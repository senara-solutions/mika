# mika#2073 — `bootstrap()` lit un état partagé de processus ; ses tests doivent poser le tier au lieu de l'espérer

**Ticket :** senara-solutions/mika#2073
**Branche :** `test/2073/mika-common-test-bootstrap-creates`
**Type :** correctif de substrat de test (comportement de production inchangé)

---

## Le défaut, et ce que la lecture du code rectifie

### 1.1 Le diagnostic du ticket est juste

`bootstrap()` (`crates/mika-common/src/home.rs:521`) appelle `AgentTier::from_env()`
en ligne 527, qui lit `MIKA_AGENT_TIER` sur l'**environnement du processus**
(`:160`). Les gabarits sont choisis par cette valeur : `tier.identity_toml()` et
`tier.soul_md()`. `DEFAULT_SOUL` porte « executive assistant », `FAMILY_SOUL` est
en français et ne la porte pas.

`test_bootstrap_creates_structure` porte `#[test]` **seul** et assère
`soul.contains("executive assistant")`. Six tests `#[serial]` du même fichier
posent ou retirent `MIKA_AGENT_TIER` sur le processus entier. `#[serial]` ne
sérialise ses porteurs **qu'entre eux** : un `#[test]` nu tourne en parallèle
d'eux. La fenêtre est réelle et étroite — entre le `set_var` et le `remove_var`
d'un test sériel, à cheval sur le `from_env()` du test nu.

Le commentaire 1/2 de samidarko clôt la démonstration : même arbre, même commit,
`failure` puis `pass` au re-run. L'issue ne dépend pas du code testé.

### 1.2 Rectification 1 — les numéros de ligne du ticket ont dérivé

Le fichier a grossi depuis le 29/08. Correspondance à l'état de la branche
(`467279bd`) :

| Ticket | Réel | Objet |
|---|---|---|
| `:790` | **`:1052`** | `test_bootstrap_creates_structure` |
| `:810` | **`:1071`** | l'assertion qui panique |
| `:522` | **`:758`** | doc-comment de `FAMILY_SOUL` |
| `:1066, :1147…` | **`:1327, :1408, :1410, :1438, :1463, :1465, :1685-1710, :1757, :1848, :1862`** | les mutateurs de `MIKA_AGENT_TIER` |

Aucune conclusion du ticket n'en est affectée. C'est noté pour que l'implémenteur
ne cherche pas une ligne 790 qui parle d'autre chose.

### 1.3 Rectification 2 — la classe déborde `home.rs`, et c'est mesuré

AC2 écrit « audit du **même fichier** », puis « le défaut est la classe, pas la
ligne 790 ». Les deux phrases divergent, et la mesure tranche en faveur de la
seconde.

`crates/mika-agent/src/well_known_agents.rs` porte **six** appels de
`bootstrap_agent()` dans des tests, aux lignes 2252, 2273, 3210, 3660, 3681,
3705. `bootstrap_agent` appelle `bootstrap` (`home.rs:395`), donc lit
`AgentTier::from_env()`. Ces six tests tournent dans le **même binaire**
(`-p mika-agent --lib`) que deux poseurs de `MIKA_AGENT_TIER=family` :
`server/mod.rs:1952` et `server/tier_guard.rs:464`, tous deux `#[serial_test::serial]`.

C'est **exactement la même course**, armée de la même façon. Elle est silencieuse
aujourd'hui parce qu'aucun de ces six tests n'assère le contenu d'un gabarit —
la même chance qui a tenu `home.rs` jusqu'au 29/08.

**Décision :** le correctif couvre les deux crates. L'argument n'est pas
l'exhaustivité pour elle-même, c'est que **la garde permanente l'impose** : le
patron maison de ce fichier (`mika2230_le_tier_a_un_seul_analyseur`) scanne tout
le workspace avec une allowlist vide, et une garde livrée avec six exceptions
documente sa propre défaite. Convertir les six est mécanique.

### 1.4 Ce qui est sensible aujourd'hui, et ce qui est une mine armée

Les huit tests nus de `home.rs` appelant un `bootstrap*` :

| L | test | assertion sensible au tier ? |
|---|---|---|
| **1052** | `test_bootstrap_creates_structure` | **OUI — `soul.contains("executive assistant")`** |
| 1075 | `test_bootstrap_does_not_overwrite_existing` | non — écrase `soul.md`, assère `"custom soul"` |
| 1093 | `test_bootstrap_sets_permissions` | non — modes 0700/0600 |
| 1138 | `test_bootstrap_agent` | non — `is_file()` / `is_dir()` |
| 1151 | `test_bootstrap_agent_rejects_invalid_name` | n'atteint jamais `bootstrap` (refus de nom) |
| 1187 | `test_migrate_to_multi_agent_from_legacy` | non — écrase `soul.md` avant d'assérer |
| 1233 | `test_migrate_to_multi_agent_idempotent` | non |
| 1288 | `test_bootstrap_fresh_install` | non — existence + `root_config.contains("global")` |

Aucun test nu n'appelle `AgentTier::from_env()` directement. Le seul candidat par
le nom, `mika2230_from_env_reads_the_extracted_parser` (`:1874`), appelle
`AgentTier::parse()`, une fonction **pure** : il est sain, et il ne faut pas le
« corriger ».

**Table vérifiée exhaustive** par scan de tous les appels `bootstrap*` du fichier.
Deux tests voisins en sortent, et il faut dire pourquoi — sinon l'implémenteur qui
refait le grep les croira ratés et perdra un aller-retour à le demander :

- `test_migrate_to_multi_agent_noop_on_fresh` (`:1256`) — n'appelle **aucun**
  `bootstrap*` : il exerce `migrate_to_multi_agent` sur un répertoire vide.
- `test_bootstrap_fresh_install_writes_narrow_skill_allowlist` (`:1324`) — appelle
  bien `bootstrap_fresh_install` (`:1332`), mais il est **déjà `#[serial]`** et
  ouvre sur un `remove_var("MIKA_AGENT_TIER")` défensif. Il est hors de la classe.
  Le convertir serait néanmoins cohérent avec U2 et supprimerait son `remove_var`
  défensif ; **laissé au jugement de l'implémenteur**, sans quoi la garde d'U5 —
  qui ne cible que les `#[test]` nus — resterait verte dans les deux cas.

**Un seul test casse aujourd'hui ; les sept autres sont des mines armées.** Leur
conversion ne répare rien maintenant et empêche le retour du défaut le jour où
quelqu'un ajoute une assertion de contenu dans `test_bootstrap_agent`. C'est ce
qui fait d'AC2 une exigence de classe et non de ligne.

### 1.5 La phrase d'AC3 répond à un fait mesuré, pas à une précaution

AC3 justifie sa phrase par « sans elle, le prochain test ajouté refera la même
chose ». Ce n'est pas une hypothèse : **c'est déjà arrivé, et la trace est dans le
fichier.** `test_bootstrap_fresh_install_writes_narrow_skill_allowlist` (`:1324`)
porte ce doc-comment, écrit sous mika#1778 :

> `#[serial]` (mika#1778): reads the default identity, which depends on
> `MIKA_AGENT_TIER` being unset/default — must not race the family-tier serial tests.

Son auteur a **vu** la course, l'a nommée correctement, et y a répondu par
`#[serial]` — pour *son* test. Le raisonnement est juste et il s'arrête là où il
fallait continuer : « must not race the family-tier **serial** tests » suppose que
les concurrents dangereux sont sériels. Les huit tests nus de §1.4 ne le sont pas,
et lisaient déjà la même variable au même moment, dix-huit mois avant que l'un
d'eux ne tire.

Deux conséquences pour la conception :

1. **AC3 est bien formulé et la phrase exacte compte.** Ce qui manquait n'était pas
   la conscience de l'état partagé — elle était présente et écrite — mais la moitié
   qui dit que `#[serial]` ne borne *que* ses porteurs. Une reformulation plus
   vague reproduirait exactement ce demi-raisonnement.
2. **La phrase seule ne suffira pas** (§2.5). Un commentaire correct existait déjà
   à un site de ce fichier et n'a pas empêché le défaut à huit autres. C'est
   l'argument mesuré pour que la garde d'U5 soit structurelle : c'est elle, et non
   le texte, que le prochain auteur de test rencontrera.

Ce constat ne change aucune unité de travail — il durcit le choix d'U4 (écrire la
phrase *avec* sa seconde moitié) et retire à U5 son air de précaution.

### 1.6 Le vecteur symétrique, pour mémoire

Trois des tests sériels font `remove_var` (`:1327`, `:1438`, `:1862`). Un test
nu qui *attendrait* `Family` serait cassé par eux. Aucun n'existe aujourd'hui.
L'injection ferme les deux directions d'un coup, sans qu'il faille les traiter
séparément.

---

## Conception

### 2.1 Le correctif : injecter le tier, ne pas ajouter un `#[serial]`

AC1 l'écrit : « la correction préférée est de rendre le tier explicite pour ce
test plutôt que d'ajouter un `#[serial]` de plus ». Trois options ont été pesées :

| Option | Verdict |
|---|---|
| **A — extraire `bootstrap_with_tier(home, tier)`, `bootstrap` devient son lecteur d'env** | **retenue** |
| B — assouplir l'assertion (« soul est l'un des trois gabarits ») | refusée : ça ne supprime pas la lecture d'env, ça la rend tolérante — le test n'atteste alors plus rien |
| C — ajouter `#[serial]` | refusée par AC1, et elle sérialise toute la suite pour un défaut qui a une correction structurelle |

L'option A est **le patron déjà posé dans ce fichier** par mika#2230 :
`AgentTier::parse` est la fonction pure, `AgentTier::from_env` est le mince
lecteur d'environnement au-dessus. Appliquer la même forme un cran plus haut est
une cohérence, pas une invention.

### 2.2 La chaîne entière, pas son maillon supérieur

`bootstrap_fresh_install` → `bootstrap_agent` → `bootstrap`. Injecter dans
`bootstrap` seul laisserait `test_bootstrap_agent` (`:1138`) et
`test_bootstrap_fresh_install` (`:1288`) lire l'environnement par le maillon
qu'ils testent — et forcerait les six tests de `well_known_agents.rs` à
contourner la fonction qu'ils exercent. Les trois maillons prennent donc une
variante `_with_tier`, et les trois fonctions historiques deviennent des lecteurs
d'un seul appel.

### 2.3 Surface API : publiques, non gated — et pourquoi

Les trois `*_with_tier` sont **publiques et non gated**. Le gating derrière
`test-utils` aurait un mérite (zéro surface de production pour deux des trois) et
deux défauts qui pèsent plus lourd :

1. `bootstrap_agent_with_tier` doit être **visible depuis `mika-agent`** pour la
   conversion des six tests de `well_known_agents.rs`. Le gating exige alors que
   `mika-agent` active `mika-common/test-utils` en dev-dependency — un couplage
   de build en plus, pour un bénéfice de surface nul.
2. Gater deux maillons sur trois d'une même chaîne crée une asymétrie qu'aucun
   lecteur ne peut deviner en arrivant sur le fichier.

`bootstrap_with_tier` a de toute façon un sens de production propre : c'est la
forme honnête de la primitive, celle qui ne dépend pas d'un état global.
Aucun appelant de production n'est anticipé et aucun n'est ajouté — le dépôt
refuse d'anticiper un consommateur, et ce plan ne le fait pas.

### 2.4 La preuve devient déterministe, et l'AC4 est livrée en plus

AC4 demande « plusieurs exécutions d'affilée, vertes ». C'est une preuve
**probabiliste** : la fenêtre de course est étroite, et N exécutions vertes ne
distinguent pas « le défaut est fermé » de « le défaut n'a pas été tiré ».

L'injection permet mieux : un **contrôle positif déterministe** — poser
`MIKA_AGENT_TIER=family` et vérifier que `bootstrap_with_tier(home, Default)`
rend quand même `DEFAULT_SOUL`. Ce test-là est légitimement `#[serial]`
(il *pose* la variable), et c'est précisément son objet : il atteste l'immunité
à l'environnement, il ne l'espère pas. Il rougit le jour où quelqu'un débranche
l'injection.

AC4 est livrée quand même, parce que le ticket la demande, et parce qu'une
exécution répétée mesure une chose que le test déterministe ne mesure pas : que
la suite entière est verte.

**Contrainte de toolchain :** `rust-toolchain.toml` pose le canal `1.93` (stable)
et `--shuffle` est `nightly` (`-Z unstable-options`). L'ordre mélangé littéral
n'est pas disponible. Le substitut qui teste la **même** propriété est de faire
varier le parallélisme (`--test-threads`), puisque c'est l'entrelacement — non
l'ordre — qui arme la course. Cela doit être écrit dans le corps de PR plutôt
que de laisser croire qu'un `--shuffle` a tourné.

### 2.5 La garde permanente doit être structurelle, et c'est démontrable

Un test comportemental **ne peut pas** attraper cette classe : la régression ne
rend aucune décision fausse, elle rend une décision non-déterministe — et un test
qui échoue une fois sur cent passe en CI. C'est la forme exacte que le dépôt
traite ailleurs par un scan de source (mika#2131 : « la régression ne rendrait pas
une décision fausse, elle la rendrait invisible »).

**Et la garde documentaire a déjà été essayée sur ce fichier, sans succès**
(§1.5) : un commentaire correct sur la course existe à `:1324` depuis mika#1778 et
n'a protégé que le test qui le porte. L'argument pour un scan n'est donc pas
seulement théorique — c'est la mesure que le texte seul n'atteint pas les sites
qu'il ne touche pas.

Garde : un test qui lit le source, trouve chaque `#[test]` **non-`#[serial]`**
dont le corps appelle `bootstrap(` / `bootstrap_agent(` /
`bootstrap_fresh_install(` / `AgentTier::from_env()`, et refuse.

**Disposition : halt-and-surface, allowlist vide.** Quand elle tire, on injecte
le tier ; on n'ajoute pas d'entrée. Modèle littéral :
`mika2230_le_tier_a_un_seul_analyseur` (`:1940`) et son contrôle de bonne foi
`mika2230_the_tier_parser_guard_fires_on_a_relapse` (`:1997`) — **les deux**, car
un détecteur vérifié par son seul vert n'est vérifié par rien.

**Note d'implémentation qui évitera une fausse piste :**
`crate::source_guard::ProductionScanner` **masque** les régions `cfg(test)`
(`source_guard.rs:117-126`). C'est l'exact inverse du besoin ici, où la cible
*est* le bloc de test. La garde lit le fichier brut. `ProductionScanner` reste le
bon outil pour la garde mika#2230 voisine ; il ne l'est pas pour celle-ci.

---

## Plan d'implémentation

### U1 — extraire l'injection du tier (`crates/mika-common/src/home.rs`)

- `pub fn bootstrap_with_tier(home_dir: &Path, tier: AgentTier) -> Result<()>` :
  le corps actuel de `bootstrap`, sans la ligne 527.
- `pub fn bootstrap(home_dir: &Path) -> Result<()>` devient
  `bootstrap_with_tier(home_dir, AgentTier::from_env())`.
- Idem pour `bootstrap_agent_with_tier` / `bootstrap_agent` et
  `bootstrap_fresh_install_with_tier` / `bootstrap_fresh_install`.
- Déplacer sur `bootstrap_with_tier` le doc-comment qui décrit le choix des
  gabarits (`:516-520`) ; laisser sur `bootstrap` la phrase qui dit qu'il lit
  `MIKA_AGENT_TIER`. Les deux faits doivent être lisibles là où ils s'appliquent.

**Comportement de production strictement inchangé** — c'est une extraction.

### U2 — convertir les huit tests de `home.rs` (AC1 + AC2)

Les huit de la table §1.4 passent à `*_with_tier(…, AgentTier::Default)`.
`test_bootstrap_agent_rejects_invalid_name` (`:1151`) est converti aussi, bien
qu'il n'atteigne jamais `bootstrap` : laisser un seul appel non converti dans le
fichier rendrait la garde d'U5 inapplicable, et une garde à exception unique est
une garde qu'on désarme à la première gêne.

### U3 — convertir les six tests de `well_known_agents.rs` (§1.3)

Lignes 2252, 2273, 3210, 3660, 3681, 3705 → `bootstrap_agent_with_tier(…, AgentTier::Default)`.
Les six sont vérifiées : ce sont les seuls appels `bootstrap*` du fichier hors du
site de production. `pre_seed_identity` (`:3209`) est un helper : la conversion y
couvre ses appelants d'un coup.

**Le septième appel, `:927`, est de production** — c'est `provision_agent` qui
appelle `bootstrap_agent` pour créer un agent bien connu. Il doit **rester** un
lecteur d'environnement : c'est le chemin par lequel `MIKA_AGENT_TIER` atteint
légitimement un agent au premier démarrage (mika#1778). Le convertir inverserait le
comportement de production, ce qu'U1 s'interdit explicitement.

Vérifier au passage qu'aucun de ces six n'assère le contenu d'un gabarit. Si l'un
le fait, **le dire dans le corps de PR** : ce serait un second défaut vivant, et
non plus une mine armée.

### U4 — écrire la raison près du code (AC3)

Deux sites, deux registres :

- **Sur `bootstrap_with_tier`**, la raison d'exister : « le tier passe par
  argument pour qu'un appelant — un test en particulier — n'ait pas à espérer un
  état de processus. »
- **Dans `mod tests`, une fois**, en tête de la section bootstrap, la phrase que
  le ticket exige mot pour mot : **`#[serial]` ne protège que des autres
  `#[serial]` ; un `#[test]` nu tourne en parallèle d'eux, et une variable
  d'environnement est un état partagé de processus.** Plus le geste correct :
  appeler la variante `_with_tier`.

**La seconde moitié de cette phrase est celle qui manquait** (§1.5) : le
doc-comment de `:1324` porte déjà la première. Écrire « attention à l'état
partagé » sans « `#[serial]` ne borne que ses porteurs » reproduirait le
demi-raisonnement qui a laissé huit tests nus en place.

Une phrase répétée à huit sites serait bruit ; la garde d'U5 est ce qui fait que
le prochain test la rencontre de toute façon.

### U5 — la garde et son contrôle de bonne foi (§2.5)

Deux tests dans `home.rs`, sous `mod tests` :

- `mika2073_no_bare_test_reads_the_tier_from_the_environment` — scan de source,
  workspace entier (modèle `:1941-1990` pour l'énumération des crates et le
  garde-fou « le scan a-t-il vraiment lu quelque chose ? »), allowlist vide,
  message nommant le défaut, sa fenêtre et le geste de correction.
- `mika2073_the_guard_fires_on_a_relapse` — contrôle de bonne foi sur des lignes
  **fabriquées**, jamais en éditant du source réel. Rechutes à attraper et
  innocents à épargner (un `#[serial]` légitime, une mention de `bootstrap` dans
  un commentaire, un appel `_with_tier`).

### U6 — le contrôle positif déterministe (§2.4)

`mika2073_an_explicit_tier_survives_a_hostile_environment` : `#[serial]`, pose
`MIKA_AGENT_TIER=family`, appelle `bootstrap_with_tier(home, AgentTier::Default)`,
assère `DEFAULT_SOUL` ; nettoie. C'est ce test qui rougit si l'injection est
débranchée — il remplace la preuve probabiliste d'AC4 par une preuve.

### U7 — documentation

`crates/mika-common/CLAUDE.md` § *Home directory* décrit le tier et `bootstrap()`.
Ajouter la paire `bootstrap` / `bootstrap_with_tier`, la raison de la séparation,
et la garde avec sa disposition halt-and-surface. Deux à quatre phrases : le
paragraphe est déjà long, et ce qui manque est la ligne qu'un futur auteur de
test doit croiser.

### U8 — preuve (AC4)

Dans le corps de PR, littéralement, avec leur sortie :

```
for i in 1 2 3 4 5; do cargo test -p mika-common --lib || break; done
cargo test -p mika-common --lib -- --test-threads=1
cargo test -p mika-common --lib -- --test-threads=16
cargo test -p mika-agent --lib
```

Et la phrase qui dit pourquoi il n'y a pas de `--shuffle` (§2.4) — annoncer un
ordre mélangé qui n'a pas eu lieu serait la forme d'attribution fausse que ce
ticket existe pour fermer.

---

## Definition of Done

- [ ] `bootstrap_with_tier` / `bootstrap_agent_with_tier` /
      `bootstrap_fresh_install_with_tier` existent ; les trois fonctions
      historiques sont leurs lecteurs d'environnement.
- [ ] Les huit tests de `home.rs` et les six de `well_known_agents.rs` passent le
      tier explicitement ; aucun `#[serial]` n'a été ajouté pour cette raison.
- [ ] La raison est écrite aux deux sites d'U4.
- [ ] La garde d'U5 est verte, son contrôle de bonne foi aussi, et son allowlist
      est vide.
- [ ] Le contrôle positif d'U6 est vert et rougit si l'injection est débranchée
      (vérifié à la main une fois, mentionné dans le corps de PR).
- [ ] `cargo test -p mika-common --lib` vert **cinq fois d'affilée**, plus une
      exécution à `--test-threads=1` et une à `--test-threads=16`.
- [ ] `cargo test -p mika-agent --lib` vert.
- [ ] `cargo clippy --all-targets` et `cargo fmt --check` propres.
- [ ] `crates/mika-common/CLAUDE.md` à jour.
- [ ] Aucun changement de comportement de production — l'extraction est à
      iso-comportement, et le corps de PR le dit.

---

## Acceptance criteria

Transcrits verbatim du corps de mika#2073.

- **AC1** — `test_bootstrap_creates_structure` ne peut plus observer un
  `MIKA_AGENT_TIER` posé par un autre test. La correction préférée est de rendre
  le tier explicite pour ce test plutôt que d'ajouter un `#[serial]` de plus :
  une assertion sur `DEFAULT_SOUL` doit dire quel tier elle suppose.

- **AC2** — Audit du même fichier : tout test qui appelle `bootstrap()` ou
  `AgentTier::from_env()` sans être sériel est traité de la même façon. Le défaut
  est la classe, pas la ligne 790.

- **AC3** — La raison est écrite près du test : `#[serial]` ne protège que des
  autres `#[serial]`, et une variable d'environnement est un état partagé de
  processus. Sans cette phrase, le prochain test ajouté refera la même chose.

- **AC4** — Preuve dans la PR : `cargo test -p mika-common --lib` exécuté
  plusieurs fois d'affilée, vert à chaque fois, et si possible avec un ordre
  d'exécution mélangé.

**Écarts assumés, et ils sont deux.** *Sur AC2* : l'audit déborde « le même
fichier » pour couvrir `well_known_agents.rs`, sur la mesure de §1.3 et sur la
seconde phrase de l'AC lui-même. *Sur AC4* : l'ordre mélangé littéral n'existe
pas sur stable 1.93 (§2.4) ; il est remplacé par une variation du parallélisme et
par le contrôle déterministe d'U6, qui prouve davantage.

---

## Hors périmètre

- **Le contenu de `DEFAULT_SOUL` et `FAMILY_SOUL`** — hors périmètre écrit du
  ticket, et corrects tous les deux.
- **Le mécanisme de tier lui-même** — il fonctionne comme prévu. Rien ici ne
  touche `AgentTier::parse`, `from_env`, la règle fail-closed de mika#2023, ni
  les deux axes `ToolsProfile` / `PersonaProfile`.
- **Les autres variables d'environnement partagées** (`MIKA_HOME`,
  `MIKA_DEPLOYMENT`) : la même classe de course existe en principe pour elles.
  Aucune n'a produit d'échec mesuré, et `MIKA_HOME` est déjà `#[serial]` à ses
  deux sites (`:984`, `:994`). **Ticket de suivi si une mesure l'exige** —
  élargir la garde à une variable sans défaut mesuré serait généraliser depuis un
  point.
- **Le test `mika2230_from_env_reads_the_extracted_parser`** (`:1874`) : son nom
  évoque `from_env`, son corps appelle `parse`, il est pur. À ne pas toucher.

---

## Risques

| Risque | Portée | Mitigation |
|---|---|---|
| L'extraction change un comportement de production | faible — c'est un déplacement de `let tier = …` d'un cran | la suite existante, qui couvre les trois tiers par les tests sériels, est conservée intacte |
| La garde d'U5 tire sur un site légitime | réelle — elle scanne tout le workspace | le contrôle de bonne foi d'U5 énumère les innocents ; si elle tire en CI, la résolution est d'injecter le tier, **jamais** d'allowlister |
| Un des six tests de `well_known_agents.rs` assère un contenu de gabarit | à établir en U3 | ce serait un second défaut vivant : le nommer dans le corps de PR plutôt que de le corriger en silence |
| Les cinq exécutions d'AC4 sont vertes sans rien prouver | certaine — c'est la nature d'une preuve probabiliste sur une fenêtre étroite | c'est pourquoi U6 existe ; AC4 est livrée par fidélité au ticket, pas comme preuve principale |
