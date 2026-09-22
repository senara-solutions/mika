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

### 1.3 Rectification 2 — la classe déborde `home.rs`, c'est mesuré, et son traitement part en suivi

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

**Décision (révisée en rev 2, finding F3) : hors périmètre de cette PR.** AC2 dit
« audit du **même fichier** ». Sa seconde phrase (« le défaut est la classe, pas
la ligne 790 ») justifie une lecture plus large, mais elle ne suffit pas à
l'autoriser : étendre l'audit à un autre crate est une **divergence de spec**, et
une divergence de spec se ratifie par l'opérateur, jamais par l'architecte ni par
le plan qui en bénéficie.

La mesure ci-dessus n'est pas perdue pour autant : **elle est le corps du ticket
de suivi** — « convertir les six `bootstrap_agent()` de `well_known_agents.rs` et
élargir le scan d'U5 au workspace » — à ouvrir avec cette PR, avec les six lignes
nommées, les deux poseurs de `MIKA_AGENT_TIER` du même binaire, et la raison pour
laquelle la course y est silencieuse aujourd'hui.

**Conséquence assumée, écrite plutôt que masquée : les six restent des mines
armées** à l'issue de cette PR, exactement comme elles le sont depuis dix-huit
mois. Rien n'est aggravé ; rien n'est réparé non plus. Et la garde d'U5 est
bornée au fichier réellement audité (§2.5) — une garde workspace-wide livrée ici
serait rouge sur six sites que cette PR n'a pas le mandat de convertir,
c'est-à-dire non livrable.

**Voie de retour, si l'opérateur ratifie.** Si Vincent étend AC2 (édition du corps
d'issue, ou commentaire édit-notice) avant l'implémentation, U3 et le scan
workspace-wide se réactivent **tels qu'ils étaient rédigés en rev 1** : aucune
autre partie de ce plan ne change, seul le périmètre rebascule. C'est pourquoi
cette section est conservée intégralement au lieu d'être supprimée.

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
  bien `bootstrap_fresh_install` (`:1332`), et il est **déjà `#[serial]`**, ouvrant
  sur un `remove_var("MIKA_AGENT_TIER")` défensif. Il est hors de la classe que la
  garde d'U5 détecte, qui ne vise que les tests non sériels.

  **Décision (rev 2, finding F2) : il est converti, et son `#[serial]` comme son
  `remove_var` sont retirés.** L'alternative — le laisser intact — est défendable
  et elle est écartée pour une raison vérifiable plutôt que par goût : une fois le
  tier injecté, son doc-comment (« must not race the family-tier serial tests »)
  devient **faux**, et un commentaire faux portant sur la course exacte que ce
  ticket ferme est un piège pire que son absence — c'est le demi-raisonnement de
  §1.5, laissé en place avec l'autorité d'un commentaire à jour.

  **Le retrait de `#[serial]` est sûr, et c'est mesuré, pas supposé.** La chaîne
  `bootstrap_fresh_install` → `bootstrap_agent` → `bootstrap` ne lit qu'**un seul**
  état de processus : `MIKA_AGENT_TIER`, via `AgentTier::from_env()`
  (`home.rs:160`). `home_dir` est un argument ; `MIKA_HOME` (`:328`) et
  `MIKA_DEPLOYMENT` (`:302`) ne sont sur aucun maillon de cette chaîne. Le tier
  injecté, il ne reste rien à sérialiser.

  **Précondition d'implémentation, et le geste si elle tombe :** si le corps du
  test a acquis d'ici là une autre lecture d'état partagé, garder `#[serial]` et
  **réécrire** le doc-comment pour nommer *cette* raison-là. Ce qui n'est pas
  permis est de conserver `#[serial]` sous son motif d'origine.

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
`bootstrap_fresh_install(` / `AgentTier::from_env()`, et refuse. Sa disposition
est écrite en propre à la section **Fire-Disposition** ci-dessous.

Modèle littéral : `mika2230_le_tier_a_un_seul_analyseur` (`:1940`) et son contrôle
de bonne foi `mika2230_the_tier_parser_guard_fires_on_a_relapse` (`:1997`) — **les
deux**, car un détecteur vérifié par son seul vert n'est vérifié par rien.

**Portée du scan, et elle est bornée (rev 2, finding F3).** Le modèle mika#2230
scanne tout le workspace ; celui-ci scanne **`crates/mika-common/src/home.rs`
seul**, parce que c'est le fichier qu'AC2 met dans le périmètre (§1.3). Ce n'est
pas une garde affaiblie par prudence mais une garde **alignée sur le mandat de sa
PR** : un scan qui refuse six sites qu'on n'a pas le droit de convertir n'est pas
plus fort, il est inlivrable — et une garde inlivrable finit allowlistée, ce que
sa disposition interdit précisément.

Deux conséquences à écrire, sans quoi un lecteur futur lui prêtera une portée
qu'elle n'a pas :

1. **Le garde-fou « le scan a-t-il vraiment lu quelque chose ? » devient plus
   nécessaire, pas moins.** Sur un fichier unique, un chemin qui dérive rend un
   scan vide et vert, indistinguable d'un arbre propre (classe mika#2205). La
   garde assère donc un nombre minimal de tests rencontrés avant de conclure.
2. **L'élargissement au workspace est le second livrable du ticket de suivi de
   §1.3**, au même commit que la conversion des six — élargir le scan avant
   convertir les sites le rendrait rouge, dans l'autre ordre il démarre vert et
   n'est pas vérifié.

**Note d'implémentation qui évitera une fausse piste :**
`crate::source_guard::ProductionScanner` **masque** les régions `cfg(test)`
(`source_guard.rs:117-126`). C'est l'exact inverse du besoin ici, où la cible
*est* le bloc de test. La garde lit le fichier brut. `ProductionScanner` reste le
bon outil pour la garde mika#2230 voisine ; il ne l'est pas pour celle-ci.

### 2.6 Le prédicat de la garde est décidé par trois mesures, pas par sa forme naïve

La formulation « chaque `#[test]` non-`#[serial]` dont le corps appelle
`bootstrap(` » est juste comme intention et fausse comme prédicat. Trois mesures
sur l'arbre à l'état de la branche la corrigent, et chacune se paie au premier run
de CI si elle est découverte à l'implémentation plutôt qu'ici.

| Mesure | Conséquence sur le prédicat |
|---|---|
| **`#[serial]` a deux orthographes** : `#[serial]` (195 occurrences) et `#[serial_test::serial]` (26) — dont `server/tier_guard.rs:443` et `:461`, c'est-à-dire **les poseurs de `MIKA_AGENT_TIER` que §1.3 nomme** | reconnaître les deux, sinon la garde déclare non-sériels 26 tests qui le sont — et une garde qui crie sur des sites corrects est une garde qu'on allowliste à la première gêne, ce qui la tue |
| **Le faux positif existe déjà dans l'arbre** : `tools/update_core_memory.rs:1093` s'appelle `test_updates_still_capped_after_bootstrap` — `bootstrap` y est dans le **nom du test**, aucun appel | le prédicat porte sur un **appel** (`bootstrap_agent(`, `bootstrap(`, `bootstrap_fresh_install(` en position d'appel), jamais sur la présence de la chaîne. Ce test est l'innocent de référence du contrôle de bonne foi d'U5 : il est réel, il est `#[tokio::test]`, et il ne doit pas tirer |
| **Les attributs de test ne sont pas que `#[test]`** : `#[tokio::test]`, `#[tokio::test(flavor = "multi_thread", …)]` (63), `#[tokio::test(start_paused = true)]` | **décision : la garde couvre `#[test]` et `#[tokio::test…]`.** La justification de rev 1 (« la surface asynchrone de `mika-agent` ») est tombée avec le bornage du scan (§2.5) ; celle qui reste est plus simple et plus solide : le prédicat répond à « ce test tourne-t-il en parallèle des autres ? », et un `#[tokio::test]` y répond oui exactement comme un `#[test]` nu. Coût : une alternance de plus dans une expression déjà écrite |

Un quatrième point suit du troisième et mérite d'être écrit : un test tokio
**multi-thread** n'est pas plus protégé qu'un test nu — `#[serial]` et le
parallélisme de `libtest` sont deux mécanismes distincts, et c'est la même
confusion que §1.5 mesure sur `#[serial]`. Le geste correct reste identique :
passer le tier.

**Ce que le bornage du scan (§2.5) change à ces trois mesures.** Elles restent
exactes et elles changent de rôle : dans `crates/mika-common/src/home.rs` seul,
mesuré à l'état de la branche, **deux des trois formes ne se rencontrent pas**.
La seconde orthographe d'attribut (`#[serial_test::serial]`) est absente — le
fichier n'a que `use serial_test::serial;` (`:980`) et dix `#[serial]` — et il n'y
a **zéro** test tokio. Ces deux branches du prédicat ne sont donc attestées que
par le contrôle de bonne foi d'U5, sur lignes fabriquées, et **pas** par une
rencontre sur l'arbre. Le dire évite qu'un relecteur lise le vert de la garde
comme une preuve qu'elles fonctionnent sur du source réel ; c'est le ticket de
suivi de §1.3, en élargissant le scan, qui les exercera pour de bon.

Un corollaire de prédicat tombe de la même mesure : la ligne `:980` contient la
chaîne `serial_test::serial` sans être un attribut. Le prédicat porte donc sur un
**attribut** (`#[…]`), jamais sur la présence de la chaîne — exactement la même
règle que pour les appels, et le même faux positif à un caractère près.

**Ce que la garde ne prétend pas faire.** Elle lit du texte, pas un AST : un appel
écrit sur plusieurs lignes, aliasé (`use home::bootstrap as b;`) ou traversant un
helper non nommé lui échappe. C'est acceptable parce que la classe visée est
l'écriture ordinaire — les huit sites nus de §1.4, son neuvième site sériel et
les six de §1.3 sont tous des appels directs sur une ligne — et parce qu'une
garde textuelle qui attrape le cas
ordinaire vaut mieux qu'une garde AST qu'on n'écrit pas. Le dire évite qu'un
lecteur futur la croie exhaustive et en tire une fausse assurance.

---

## Fire-Disposition

Ce plan livre deux détecteurs en U5 (la garde de scan et son contrôle de bonne
foi) et un troisième en U6 (le contrôle positif déterministe). Ce que chacun fait
**quand il tire** est écrit ici plutôt que laissé au premier qui le rencontrera
en CI.

| Détecteur | Ce que son rouge signifie | Disposition |
|---|---|---|
| `mika2073_no_bare_test_reads_the_tier_from_the_environment` (U5) | un test non sériel lit `MIKA_AGENT_TIER` via un `bootstrap*` — la mine de §1.4 est réarmée | **(c) halt-and-surface, allowlist vide** |
| `mika2073_the_guard_fires_on_a_relapse` (U5) | le prédicat de la garde ci-dessus est cassé : il ne voit plus une rechute, ou il crie sur un innocent | **(c) halt-and-surface** — réparer le prédicat, jamais assouplir le contrôle |
| `mika2073_an_explicit_tier_survives_a_hostile_environment` (U6) | l'injection du tier est débranchée : `*_with_tier` relit l'environnement | **(c) halt-and-surface** — restaurer l'injection, jamais relâcher l'assertion |

**Le mécanisme de la garde d'U5, en clair.** Elle **refuse** — un `panic!` de test,
pas un `warn!`, pas un compteur. Son message nomme trois choses : (1) le défaut
(« ce test lit `MIKA_AGENT_TIER` sur l'environnement du processus »), (2) sa
fenêtre (« `#[serial]` ne borne que ses porteurs ; un test non sériel tourne en
parallèle d'eux »), (3) **le geste** — appeler la variante `_with_tier` en passant
le tier attendu. Le message dit aussi ce qui n'est pas un geste correct : ajouter
un `#[serial]`, ou ajouter une entrée d'allowlist.

**Allowlist vide, et livrée vide.** Il n'y a pas de constante d'exceptions à
remplir : la garde n'en a pas. C'est le patron littéral de
`mika2230_le_tier_a_un_seul_analyseur` dans ce même fichier, et la raison est la
même — une garde livrée avec des exceptions documente sa propre défaite, et une
garde dont l'allowlist existe est une garde qu'on allonge à la première gêne
plutôt que de réparer le site. Quand elle tire, **on injecte le tier.**

**Pourquoi pas les deux autres dispositions.** *Auto-réparation* : réécrire le
site de test à la volée est exclu — le geste correct exige de savoir **quel** tier
le test suppose, ce qu'aucune machine ne peut décider à sa place, et un choix par
défaut planté silencieusement dans une assertion est le contraire de ce que ce
ticket ferme. *Tolérance avec journalisation* : un scan qui se contente de
prévenir laisse la mine armée, et cette classe de régression ne rend aucune
décision fausse — elle la rend non-déterministe (§2.5), donc un avertissement
qu'un CI vert accompagne ne sera jamais lu. Halt-and-surface est la seule des
trois qui oppose quelque chose à un test qui passe une fois sur cent.

**Ce qu'elle ne couvre pas, pour que son vert soit lu correctement.** Sa portée
est `crates/mika-common/src/home.rs` (§2.5) ; ses branches `#[tokio::test…]` et
`#[serial_test::serial]` ne rencontrent aujourd'hui aucun site dans ce fichier
(§2.6) ; et elle lit du texte, pas un AST (§2.6, dernier paragraphe). Son vert
atteste le fichier audité, rien de plus large.

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

### U2 — convertir les neuf tests de `home.rs` (AC1 + AC2)

Les huit de la table §1.4 passent à `*_with_tier(…, AgentTier::Default)`.
`test_bootstrap_agent_rejects_invalid_name` (`:1151`) est converti aussi, bien
qu'il n'atteigne jamais `bootstrap` : laisser un seul appel non converti dans le
fichier rendrait la garde d'U5 inapplicable, et une garde à exception unique est
une garde qu'on désarme à la première gêne.

**Neuvième site, décidé en rev 2 (F2) :**
`test_bootstrap_fresh_install_writes_narrow_skill_allowlist` (`:1324`) est converti
lui aussi, et il perd **trois** choses ensemble — son `remove_var` défensif, son
`#[serial]`, et le doc-comment qui les motivait. Les trois tombent pour la même
raison et doivent tomber au même commit : une fois le tier passé par argument, le
test ne lit plus aucun état de processus (démonstration en §1.4), donc le
`remove_var` ne protège de rien, le `#[serial]` ne sérialise rien, et le
commentaire décrit une course qui n'existe plus. Remplacer le doc-comment par
une ligne disant pourquoi le test n'a plus besoin d'être sériel — c'est le seul
site du fichier où la démonstration §1.5 a une conclusion visible.

### U3 — les six tests de `well_known_agents.rs` : ticket de suivi, pas cette PR (§1.3)

**Rev 2 (F3) : cette unité n'est pas exécutée ici.** L'audit reste borné au
« même fichier » d'AC2. Le livrable d'U3 devient l'**ouverture d'un ticket de
suivi**, dont le corps est §1.3 : les six lignes (2252, 2273, 3210, 3660, 3681,
3705), les deux poseurs de `MIKA_AGENT_TIER` du même binaire
(`server/mod.rs:1952`, `server/tier_guard.rs:464`), la raison pour laquelle la
course y est silencieuse aujourd'hui, et **les deux gestes indissociables** :
convertir les six *et* élargir le scan d'U5 au workspace, dans cet ordre et au
même commit (§2.5).

Le matériel ci-dessous est conservé parce qu'il est vérifié et qu'il sera le
contenu de ce ticket — pas parce que cette PR l'exécute :

- Les six sont les seuls appels `bootstrap*` du fichier hors du site de
  production. `pre_seed_identity` (`:3209`) est un helper : la conversion y
  couvre ses appelants d'un coup.
- **Le septième appel, `:927`, est de production** — c'est `provision_agent` qui
  appelle `bootstrap_agent` pour créer un agent bien connu. Il doit **rester** un
  lecteur d'environnement : c'est le chemin par lequel `MIKA_AGENT_TIER` atteint
  légitimement un agent au premier démarrage (mika#1778). Le convertir
  inverserait le comportement de production, ce qu'U1 s'interdit explicitement.
- Vérifier alors qu'aucun des six n'assère le contenu d'un gabarit. Si l'un le
  fait, ce serait un **second défaut vivant** et non plus une mine armée — à dire
  dans le corps du ticket, car cela en changerait la priorité.

**Si l'opérateur ratifie l'extension d'AC2 avant l'implémentation**, cette unité
redevient exécutable telle qu'elle était rédigée en rev 1, et le scan d'U5 part
workspace-wide. Le corps de PR doit alors le dire, en citant la ratification.

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

- `mika2073_no_bare_test_reads_the_tier_from_the_environment` — scan de source
  **borné à `crates/mika-common/src/home.rs`** (§2.5 ; modèle `:1941-1990` pour la
  lecture du fichier et pour le garde-fou « le scan a-t-il vraiment lu quelque
  chose ? », qui devient ici load-bearing puisqu'un chemin qui dérive rendrait le
  scan vide et vert). Allowlist vide, message nommant le défaut, sa fenêtre et le
  geste de correction ; disposition en section **Fire-Disposition**. Prédicat selon
  §2.6 : attributs `#[test]` **et** `#[tokio::test…]` ; `#[serial]` reconnu sous
  ses **deux** orthographes ; détection sur un **appel** et sur un **attribut**,
  jamais sur la présence d'une chaîne.
- `mika2073_the_guard_fires_on_a_relapse` — contrôle de bonne foi sur des lignes
  **fabriquées**, jamais en éditant du source réel. Rechutes à attraper : le
  `#[test]` nu appelant `bootstrap(`, et son jumeau `#[tokio::test]`. Innocents à
  épargner, dont quatre sont **mesurés dans l'arbre** plutôt qu'imaginés : un
  `#[serial]` légitime, **un `#[serial_test::serial]`** (`tier_guard.rs:443`), **un
  `use serial_test::serial;`** (`home.rs:980` — une chaîne qui n'est pas un
  attribut), **un nom de test contenant `bootstrap` sans appel**
  (`update_core_memory.rs:1093`), une mention dans un commentaire, un appel
  `_with_tier`. Deux de ces formes ne se rencontrent pas dans le fichier scanné
  (§2.6) : ce contrôle est leur **seule** attestation, ce qui est exactement
  pourquoi il ne peut pas être allégé.

**Ordre d'exécution :** écrire la garde **avant** U2 et la regarder tirer sur les
**huit** sites réels — les huit tests nus de la table §1.4 ; le neuvième (`:1324`)
est sériel et la garde ne le voit pas, ce qui n'enlève rien à sa conversion.
Une garde écrite après la conversion démarre verte, et un détecteur dont on n'a
jamais vu le rouge sur du vrai source n'est pas vérifié — le contrôle de bonne foi
couvre la logique du prédicat, pas son branchement sur l'arbre. C'est la reprise
du raisonnement de §2.5 appliquée à l'ordre des gestes.

### U6 — le contrôle positif déterministe (§2.4)

`mika2073_an_explicit_tier_survives_a_hostile_environment` : `#[serial]`, pose
`MIKA_AGENT_TIER=family`, appelle `bootstrap_with_tier(home, AgentTier::Default)`,
assère `DEFAULT_SOUL` ; nettoie. C'est ce test qui rougit si l'injection est
débranchée — il remplace la preuve probabiliste d'AC4 par une preuve.

### U7 — documentation

`crates/mika-common/CLAUDE.md` § *Home directory* décrit le tier et `bootstrap()`.
Ajouter la paire `bootstrap` / `bootstrap_with_tier`, la raison de la séparation,
et la garde avec sa disposition halt-and-surface **et sa portée bornée à
`home.rs`**, en nommant le ticket de suivi de §1.3 — sans quoi le prochain
lecteur lira le vert de la garde comme une couverture du workspace. Deux à
quatre phrases : le paragraphe est déjà long, et ce qui manque est la ligne
qu'un futur auteur de test doit croiser.

### U8 — preuve (AC4)

Dans le corps de PR, littéralement, avec leur sortie :

```
for i in 1 2 3 4 5; do cargo test -p mika-common --lib || break; done
cargo test -p mika-common --lib -- --test-threads=1
cargo test -p mika-common --lib -- --test-threads=16
cargo test -p mika-agent --lib
```

La dernière ligne est une **non-régression**, pas une preuve de conversion : U3
ne convertissant plus rien dans ce crate (rev 2, §1.3), elle atteste seulement
que l'extraction d'U1 ne casse pas les consommateurs de `mika-common`. Le corps
de PR doit le dire ainsi — laisser croire que `mika-agent` a été audité serait
l'attribution fausse que ce ticket existe précisément pour fermer.

Et la phrase qui dit pourquoi il n'y a pas de `--shuffle` (§2.4) — annoncer un
ordre mélangé qui n'a pas eu lieu relève de la même faute.

Le corps de PR porte enfin **le lien du ticket de suivi de §1.3**, ouvert avec la
PR : c'est ce qui rend l'audit borné traçable plutôt que silencieusement partiel.

---

## Definition of Done

- [ ] `bootstrap_with_tier` / `bootstrap_agent_with_tier` /
      `bootstrap_fresh_install_with_tier` existent ; les trois fonctions
      historiques sont leurs lecteurs d'environnement.
- [ ] Les **neuf** tests de `home.rs` passent le tier explicitement ; aucun
      `#[serial]` n'a été ajouté pour cette raison, et celui de `:1324` a été
      **retiré** avec son `remove_var` et son doc-comment devenu faux (§1.4, F2).
- [ ] Le **ticket de suivi de §1.3** est ouvert (six sites de
      `well_known_agents.rs` + élargissement du scan), son lien est dans le corps
      de PR, et le corps dit que l'audit de cette PR est borné à `home.rs`
      conformément à AC2 verbatim.
- [ ] La raison est écrite aux deux sites d'U4.
- [ ] La garde d'U5 est verte, son contrôle de bonne foi aussi, et son allowlist
      est vide. Son prédicat couvre `#[tokio::test…]` et les deux orthographes de
      `#[serial]` (§2.6), sa portée est `crates/mika-common/src/home.rs`, son
      garde-fou « le scan a-t-il lu quelque chose ? » est en place, et elle a été
      **vue rouge sur les huit sites réels** avant U2.
- [ ] La section **Fire-Disposition** du plan correspond à ce qui est livré : le
      message de la garde nomme le défaut, sa fenêtre et le geste, et ne propose
      ni `#[serial]` ni allowlist.
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

**Écart assumé, et il n'y en a plus qu'un** (rev 2). *Sur AC4* : l'ordre mélangé
littéral n'existe pas sur stable 1.93 (§2.4) ; il est remplacé par une variation
du parallélisme et par le contrôle déterministe d'U6, qui prouve davantage. Le
corps de PR le dit, plutôt que de laisser croire qu'un `--shuffle` a tourné —
c'est une substitution technique documentée, pas une réduction de l'exigence.

**Sur AC2, il n'y a plus d'écart.** Rev 1 étendait l'audit à
`well_known_agents.rs` ; c'était une divergence de spec que l'architecte ne peut
pas ratifier (finding F3). L'audit est donc **borné au même fichier**, mot pour
mot comme l'AC l'écrit, et la mesure qui motivait l'extension part en ticket de
suivi (§1.3, U3) au lieu d'être exécutée sans mandat. La seconde phrase d'AC2
(« le défaut est la classe, pas la ligne 790 ») reste pleinement honorée **à
l'intérieur** du fichier : neuf sites convertis, dont huit qui ne cassent pas
aujourd'hui, plus une garde structurelle.

**Ce qu'un opérateur peut vouloir changer, et comment.** Si Vincent juge que
l'audit doit couvrir les deux crates dans cette PR, le geste est d'étendre AC2
sur le corps de l'issue (ou de poser un commentaire édit-notice) : U3 et le scan
workspace-wide redeviennent alors exécutables sans réécrire une ligne de ce plan.

---

## Hors périmètre

- **Les six `bootstrap_agent()` de test de `well_known_agents.rs`** et
  l'élargissement du scan d'U5 au workspace — mesurés, réels, et **hors du
  mandat d'AC2** (§1.3, F3). Ticket de suivi ouvert avec cette PR, dont §1.3 est
  le corps. Ils restent des mines armées d'ici là, ce que le corps de PR dit.
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
| La garde d'U5 tire sur un site légitime | **mesurée, pas hypothétique** — trois faux positifs existent dans l'arbre (§2.6) | le prédicat de §2.6 les exclut par construction ; le contrôle de bonne foi d'U5 les porte comme innocents de référence. Si elle tire quand même en CI, la résolution est d'injecter le tier ou de corriger le prédicat, **jamais** d'allowlister |
| Un des six tests de `well_known_agents.rs` assère un contenu de gabarit | à établir dans le **ticket de suivi**, plus dans cette PR (F3) | ce serait un second défaut vivant : c'est la vérification qui ouvre ce ticket, et elle en fixerait la priorité |
| L'audit borné laisse les six mines armées et personne n'ouvre le ticket de suivi | **réelle** — c'est le mode de panne normal d'un report | le ticket est une case du DoD et son lien est exigé dans le corps de PR ; un report qui ne laisse pas de trace est un abandon, et c'est précisément ce que la traçabilité demandée par F3 protège |
| La garde bornée à un fichier est lue comme une couverture du workspace | **réelle** — un vert ne dit pas sa portée | la portée est écrite à trois endroits (§2.5, Fire-Disposition, `CLAUDE.md` via U7) et le message de la garde ne promet rien au-delà du fichier qu'elle lit |
| Les cinq exécutions d'AC4 sont vertes sans rien prouver | certaine — c'est la nature d'une preuve probabiliste sur une fenêtre étroite | c'est pourquoi U6 existe ; AC4 est livrée par fidélité au ticket, pas comme preuve principale |

---

## Revision history

- **rev 2 (2026-09-20)** — révision sur les trois findings de la première passe
  architecte (`Disposition: ITERATE`) :
  - **F1 (Fire-Disposition absente)** : ajout d'une section `## Fire-Disposition`
    de premier niveau, nommant **(c) halt-and-surface, allowlist vide** pour la
    garde d'U5 et halt-and-surface pour les deux autres détecteurs (contrôle de
    bonne foi d'U5, contrôle positif d'U6), avec le mécanisme — refus par `panic!`
    et message nommant le défaut, sa fenêtre et le geste (injecter le tier, jamais
    `#[serial]`, jamais allowlister) — le refus des deux autres dispositions
    (auto-réparation, tolérance journalisée), et ce que le vert de la garde
    n'atteste pas. §2.5 y renvoie au lieu de porter la disposition en ligne.
  - **F2 (décision reportée à l'implémenteur)** : tranché en §1.4 et U2.
    `test_bootstrap_fresh_install_writes_narrow_skill_allowlist` (`:1324`) **est
    converti**, et perd au même commit son `remove_var` défensif, son `#[serial]`
    et son doc-comment — lequel deviendrait faux après injection, ce qui est le
    piège que §1.5 mesure. Sûreté du retrait de `#[serial]` établie par mesure et
    non supposée : la chaîne `bootstrap_fresh_install` → `bootstrap_agent` →
    `bootstrap` ne lit qu'un état de processus (`MIKA_AGENT_TIER`, `home.rs:160`),
    `MIKA_HOME` (`:328`) et `MIKA_DEPLOYMENT` (`:302`) étant hors chaîne. Une
    précondition d'implémentation et son geste de repli sont écrits.
  - **F3 (écart AC2 non ratifié)** : l'audit est **borné à `home.rs`**,
    verbatim AC2. U3 ne convertit plus les six sites de `well_known_agents.rs` et
    devient l'ouverture d'un **ticket de suivi** dont §1.3 est le corps ; le scan
    d'U5 est borné au même fichier (une garde workspace-wide serait rouge sur six
    sites hors mandat, donc inlivrable) ; l'écart AC2 disparaît de la section
    *Acceptance criteria*, qui ne porte plus qu'un écart (AC4, `--shuffle`
    indisponible sur stable 1.93). La voie de retour est écrite : si l'opérateur
    étend AC2, U3 et le scan workspace-wide se réactivent tels qu'ils étaient
    rédigés en rev 1, sans réécriture du plan.
  - Conséquences mécaniques propagées : §2.6 (la justification tokio change de
    fondement ; deux branches du prédicat ne sont plus attestées que par le
    contrôle de bonne foi, car `home.rs` n'a aucun test tokio et aucun
    `#[serial_test::serial]` en position d'attribut — seulement le `use` de
    `:980`, d'où la règle « attribut, jamais chaîne »), U5 (« quatorze sites
    réels » → **huit**), U7, U8 (`cargo test -p mika-agent --lib` reclassé en
    non-régression), *Definition of Done*, *Hors périmètre*, *Risques* (deux
    risques ajoutés : le ticket de suivi jamais ouvert, la garde bornée lue comme
    une couverture du workspace).
- **rev 1 (2026-09-20)** — plan initial.
