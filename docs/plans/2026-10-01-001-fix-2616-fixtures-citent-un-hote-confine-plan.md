# Une fixture de grounding cite un hôte fictif, jamais un hôte confiné (mika#2616)

**Ticket :** senara-solutions/mika#2616
**Source :** main rouge depuis `41a4a20e` (merge de #2614 / mika#1984, 2026-10-01
02:54:59Z). Deux jobs CI en échec sur toute PR ouverte ; #2613 et #2615 sont
UNSTABLE pour cette seule raison.
**Type :** correction de fixtures d'éval + une ligne de prose dans un plan livré.
**Périmètre :** `crates/mika-agent/tests/eval/grounding_regressions/**` (un test,
un fixture JSON) et `docs/plans/2026-09-30-002-…-plan.md` (une ligne).
**Aucun changement de production** — aucune ligne de `crates/*/src`, aucun prompt,
aucune migration, aucune variable d'environnement, aucune clé de configuration.
**Aucun élargissement du lint, aucune entrée d'allowlist** (refus 3 du ticket, et
le § *R2* ci-dessous donne au refus un motif plus solide que celui du ticket).
**Classe :** casse la boucle (tier 1) — main rouge ⇒ aucune PR ne peut être CLEAN.

> **Le slug de ce fichier ne porte volontairement pas `2614`**, alors que le nom
> de la branche le porte. Le tier 1 de `_find_issue_plan` apparie
> `*-<issue>-*-plan.md` sur **toute suite de 4 chiffres délimitée par des
> tirets** — son propre commentaire nomme ce faux positif et cite un incident
> (mika#2038) — et un slug portant le numéro d'un autre ticket en fabrique un de
> plus gratuitement. La réfutation positionnelle de mika#2020 l'attraperait ;
> tolérer un faux positif n'est pas une licence pour en créer.

**Aucun détecteur n'est livré** — le gate mika#2306 est donc **N/A**, et c'est
établi plutôt que supposé : le détecteur de cette classe existe déjà
(`scripts/verify-egress-uniqueness.sh`, job `egress-uniqueness-lint`), il tourne
sur chaque PR, et **c'est lui qui a rougi**. En livrer un second sur le même
prédicat créerait deux sites libres de diverger — la classe que `grooming_marker`
a dû refermer une fois (mika#2158). Le § *Hors périmètre* nomme ce que ce travail
ne protège pas.

---

## Constat, reproduit sur la branche

`bash scripts/verify-egress-uniqueness.sh` rend **6 violations**, toutes sur le
même motif — l'hôte confiné de `egress_fetch` (mika#1969), que le lint dérive des
entrées `confined = true` de `docs/egress/egress-manifest.toml` depuis mika#2408
AC5 :

| # | site | nature |
|---|---|---|
| 1 | `mixed_verification_testimony_as_rule.rs:86` | prose de la réponse mock, test 1 |
| 2 | `mixed_verification_testimony_as_rule.rs:118` | argument de `VerifiedRule(…)`, test 1 |
| 3 | `mixed_verification_testimony_as_rule.rs:151` | prose de la réponse mock, test 2 |
| 4 | `mixed_verification_testimony_as_rule.rs:176` | argument de `VerifiedRule(…)`, test 2 |
| 5 | `fixtures/mixed_verification_testimony_as_rule_pre_fix.json:6` | champ `pre_fix_response` |
| 6 | `docs/plans/2026-09-30-002-…-plan.md:216` | table *Ce qui est refusé* |

Le second job, `egress-manifest-lint`, échoue par **la même cause** : son étape N1
rejoue `verify-egress-uniqueness.sh`. Un seul correctif verdit les deux.

---

## Trois rectifications que la lecture du code impose au ticket

Elles sont le premier livrable : sans elles, deux des quatre attendus du ticket
sont inexécutables comme écrits.

### R1 — les backticks n'aident pas, et la question est tranchée

L'attendu 2 propose « ou le mettre entre backticks si le lint les ignore — **à
vérifier** ». Vérifié : **non**. Le prédicat du lint est
`grep -rnF "$pat" "$REPO_ROOT/docs/"` — littéral (`-F`), par ligne, **sans aucune
notion de backtick ni de bloc clôturé**. Une mention entre backticks est un hit
exactement comme une prescription. La seule issue pour la ligne 216 est donc de
**ne plus porter le littéral**, et c'est ce que prescrit R6 ci-dessous.

Conséquence pratique immédiate, et elle porte sur **ce plan-ci** : voir le
§ *Contrainte auto-référentielle*.

### R2 — la doctrine citée par le refus 3 est hors contexte ; le refus tient quand même

L'attendu 3 refuse d'ajouter ces fichiers à `AUTHORIZED_PATHS` en citant la
doctrine du fichier : « If you find yourself adding an allowlist … fix that
instead ». Deux faits corrigent cette lecture :

1. **Cette phrase ne parle pas de la liste en général.** Elle vit aux lignes
   35-40 de `verify-egress-uniqueness.sh` et porte sur **un sujet précis** : la
   forme de délégation du builtin `fetch_url`, qui ne nomme aucun hôte parce
   qu'il délègue à `POST /internal/fetch`. « If you find yourself adding an
   allowlist **for the fetch builtin**, the delegation shape has regressed. »
2. **`AUTHORIZED_PATHS` porte déjà deux entrées de très exactement cette classe**
   (lignes 126-131), avec leur commentaire : *« Test fixtures (mika#1970) —
   grounding_regressions eval scenarios use [l'hôte] as prose payload text in a
   mock LLM response; the tests never egress. »* Ce sont
   `mixed_verification_qualification.rs` — le **prédécesseur direct** du scénario
   fautif — et `grounding_assertions/mod.rs`.

Donc l'affirmation « cette liste est réservée aux sites qui réalisent ou
documentent le substrat » est **fausse aujourd'hui**. Le dire importe, parce
qu'un relecteur qui ouvre le fichier trouve le contre-exemple en deux lignes et
pourrait en conclure que le refus est arbitraire.

**Le refus tient néanmoins, et sur deux motifs plus solides :**

- *Pour les fixtures* — la liste croîtrait d'une entrée par futur scénario de
  grounding citant un hôte réel, et chaque entrée est une **exemption permanente
  sur un fichier dont le contenu peut ensuite changer sous elle**. Corriger la
  fixture la rend propre **intrinsèquement** : il n'y a plus rien à exempter.
- *Pour le plan* — le critère des trois plans déjà allowlistés (lignes 106, 111,
  116) est **explicite** dans leur commentaire : ils *documentent le substrat*
  (« describes what the consumer-side migration removes; must cite the
  pre-migration Brave URL to document the fix »). Le plan de mika#1984 documente
  une taxonomie de grounding, **pas l'egress**. Il ne satisfait pas le critère.

C'est aussi ce qui rend le suivi du § *Hors périmètre* intéressant : convertir les
deux fixtures mika#1970 et **retirer** leurs deux entrées rendrait la phrase du
ticket vraie, au lieu de la laisser fausse.

### R3 — le ticket liste 5 sites, le lint en rend 6

Le corps du ticket écrit `…rule.rs:118 / :151 / :176` et **omet la ligne 86**.
Un implémenteur qui suit la lettre du ticket corrige cinq sites, relance le lint,
et le trouve **toujours rouge** sur un sixième — sur un ticket dont la classe est
« casse la boucle ». La table du § *Constat* est la liste exhaustive, obtenue du
lint et non du ticket.

---

## Ce que le test épingle réellement — mesuré, pas supposé

L'attendu 1 affirme que « le nom de domaine réel n'est pas ce que le test
épingle ». C'est exact, et la lecture de
`crates/mika-agent/tests/eval/grounding_assertions/mod.rs:430-481` l'établit
plutôt que de l'espérer. `assert_per_line_verification_qualification` ne cherche
que **deux choses** :

1. le **nom de l'élément** (`"25 €"`, `"8 semaines"`,
   `"Date de dernière mise à jour"`) dans le texte de la réponse ;
2. les **marqueurs de tier** de la table `TIER_MARKERS` — `[vérifié:`,
   `[source non probante`, `[non vérifié` — dans une fenêtre bornée de 200 octets
   après ce nom.

La chaîne portée par `VerifiedRule(&str)` / `SourceNotProbative(&str)` **n'est
jamais recherchée** : elle n'apparaît que dans le message de `panic!`, pour
l'attribution. Trois corroborations indépendantes :

- Le corps de la fonction (lignes 439-481) ne lit `tier` que par `tier.rank()` et
  ne déréférence jamais sa charge.
- Les tests unitaires du module lui-même (lignes ~797 à ~951) écrivent **déjà**
  `VerificationTier::VerifiedRule("page CNI officielle")`, **sans l'hôte**. La
  forme courte est donc l'idiome en vigueur dans l'arbre ; seul le fichier de
  scénario portait le domaine.
- La ligne 216 du plan mika#1984 — celle qu'il faut corriger — **dit exactement
  cela** : elle refuse « vérifier que la source déclarée apparaît dans le tag »
  parce que ce serait « une nouvelle sévérité sur un tier existant, non
  demandée ». L'auteur le savait.

**Conséquence : l'hôte est décoratif sur les six sites.** Le remplacer ne retire
aucun pouvoir d'assertion, et ne demande aucune compensation.

### Le fixture JSON n'est lu par aucun code

`grep -rn "testimony_as_rule" --include='*.rs' crates/` ne rend que des mentions
en commentaire de documentation. Le fichier est un artefact **gelé et
documentaire** (convention du README de `grounding_regressions/`) ; les réponses
du test sont des `text_response(…)` **en ligne**. L'éditer n'a aucun effet
d'exécution.

**Et cela ne contredit pas la doctrine « fixtures are frozen, not refreshed ».**
Ce qui est gelé ici est la **forme de la fabrication** — un `[vérifié:` apposé à
un témoignage — et non l'hôte. Le champ `provenance` du fixture le dit lui-même :
le contenu est déjà un *« Neutral restatement of the class … deliberately NOT
re-enacted here »*. L'hôte n'a jamais été une donnée mesurée, c'est de la prose
inventée — **on ne falsifie pas une mesure en éditant un nom qui n'en était pas
une.**

---

## L'hôte de remplacement, et pourquoi il est immune par construction

`demarches-exemple.invalid`

Trois propriétés, dans l'ordre de leur poids :

1. **Il ne peut jamais devenir confiné.** `.invalid` est réservé par la **RFC 2606
   §2** et n'est jamais délégué : aucun hôte sous ce TLD ne peut être une
   destination d'egress réelle, donc aucun ne peut légitimement entrer dans
   `egress-manifest.toml` avec `confined = true`. Le lint dérivant ses motifs de
   ce fichier (mika#2408 AC5), la fixture devient **structurellement** hors de sa
   population — pas hors par chance. C'est la doctrine que l'en-tête du lint cite
   pour lui-même : *« construct the incapacity, don't promise the restraint. »*
2. **C'est déjà l'idiome de l'arbre**, pas une invention : `gateway.invalid`
   (`skills/builtin_handlers.rs:6400`), `db.invalid`
   (`tests/eval/test_mcp_secret_boundary_2281.rs:66`).
3. **Il ne contient aucun des cinq motifs dérivés** (l'upstream de recherche de
   `egress_search`, l'hôte confiné de `egress_fetch` et ses trois
   `confined_hosts` frères) — vérifiable par le lint lui-même, qui est le juge.
   Les cinq se lisent par
   `python3 -B scripts/lib/egress_manifest_lint.py --confined-hosts .`, qui est
   la source dont le lint les dérive ; **ce plan ne les énumère pas en clair**,
   pour la raison du § *Contrainte auto-référentielle* — une première rédaction
   l'a fait et a fait rougir `verify-egress-uniqueness.sh` sur ce fichier même.

Il est « manifestement fictif » au sens de l'attendu 1, et reste lisible dans un
scénario d'administration française.

**Forme unique sur les six sites :**

```text
page CNI officielle service-public[.]fr   →   page CNI officielle demarches-exemple.invalid
```

Une seule chaîne, donc une seule vérification par grep, et aucune divergence
cosmétique entre la prose et l'argument de `VerifiedRule(…)`.

---

## Les six éditions

> Les cinq premières sont la **même** substitution. Elles se font à l'outil
> `Edit` (`sed -i` est refusé par la politique de permissions).

**R1 à R4 — `crates/mika-agent/tests/eval/grounding_regressions/mixed_verification_testimony_as_rule.rs`**

| ligne | contexte |
|---|---|
| 86 | `[vérifié: page CNI officielle …]` dans le `text_response` de `test_testimony_qualified_as_source_not_probative` |
| 118 | `VerificationTier::VerifiedRule("page CNI officielle …")`, même test |
| 151 | même tag dans le `text_response` de `test_regression_testimony_tagged_as_rule` |
| 176 | `VerificationTier::VerifiedRule("page CNI officielle …")`, même test |

**R5 — `…/grounding_regressions/fixtures/mixed_verification_testimony_as_rule_pre_fix.json`**

Champ `pre_fix_response`, une occurrence. Les champs `description`, `incident`,
`fabrication_class`, `provenance` et `note` sont **inchangés** : ils portent la
classe de fabrication, qui est ce que le fixture gèle.

**R6 — `docs/plans/2026-09-30-002-test-1984-taxonomie-trois-etats-niveau-de-preuve-plan.md:216`**

Ligne de la table *Ce qui est refusé*, colonne motif. Remplacer l'unique
occurrence entre backticks par `demarches-exemple.invalid/…/F1050`. **L'argument
est préservé mot pour mot** : il lui faut « la même source nommée autrement, sous
forme d'URL », et la forme d'URL survit au changement d'hôte.

### Ce qu'il ne faut PAS éditer

- **`service-public` sans TLD.** La ligne 22 du même `.rs` (*« a service-public
  page, an `Fxxxxx` reference »*) et les lignes homologues de
  `grounding_assertions/mod.rs` ne portent **pas** le littéral `…fr` et ne sont
  **pas** des hits. Les toucher est du bruit, et élargirait un correctif tier 1.
- **Les deux fichiers déjà allowlistés** (`mixed_verification_qualification.rs`,
  `grounding_assertions/mod.rs` — 5 occurrences au total). Ils ne font pas rougir
  le lint. Voir le suivi au § *Hors périmètre*.

---

## Le risque unique : l'arithmétique de la fenêtre de 200 octets

La substitution est **plus longue de 8 octets** (17 → 25). La fenêtre de
`QUALIFICATION_WINDOW_BYTES = 200` est bornée et non limitée à la ligne, et le
doc-comment du helper avertit que l'ordre des éléments est « load-bearing » pour
cette raison. L'analyse, élément par élément :

| élément | tier | marqueur requis | distance | marqueurs interdits en aval |
|---|---|---|---|---|
| `25 €` | `VerifiedRule` (rang 0) | `[vérifié:` | ~1 octet | **aucun** (rang 0 n'interdit rien) |
| `8 semaines` | `SourceNotProbative` (rang 1) | `[source non probante` | ~1 octet | rang 0 — absent en aval |
| `Date de dernière mise à jour` | `SnippetOnly` (rang 2) | `[non vérifié` | ~22 octets | rangs 0 et 1 — absents en aval |

Les +8 octets vivent **dans la première puce**, donc **en amont** des fenêtres de
l'élément 2 et de l'élément 3 : ils décalent des positions sans changer la
composition d'aucune fenêtre. Chaque marqueur requis est adjacent à son élément,
très en deçà de 200. Et `[non vérifié` ne contient pas `[vérifié:` — c'est le
crochet ouvrant qui l'assure, propriété que le doc-comment du helper pose
explicitement et qu'un test du module (`tier_markers_are_pairwise_disjoint`)
refuse de laisser casser.

**La substitution est donc neutre pour l'assertion.** La vérification V2 la
mesure plutôt que de s'en contenter.

### Le contrôle négatif ne dépend pas du fixture édité

Les tests unitaires de `grounding_assertions/mod.rs` (~lignes 790-960) épinglent
le refus du helper sur leurs **propres** fixtures, qui portent déjà la forme
courte sans hôte. Le contrat du helper — *« a response may under-claim its
evidence tier, never over-claim it »* — reste donc pinné **indépendamment** de ce
correctif. C'est une propriété, pas une coïncidence, et c'est ce qui rend cette
édition peu risquée.

---

## Contrainte auto-référentielle : ce plan est dans la population du lint

`verify-egress-uniqueness.sh` grep **tout `docs/`**, donc ce fichier-ci. Il ne
porte volontairement le littéral nulle part : l'hôte est désigné par périphrase
(« l'hôte confiné de `egress_fetch` ») ou écrit `service-public[.]fr`, forme que
`grep -F` ne peut pas apparier.

Corollaire utile pour l'implémenteur — une commande qui **trouve** le littéral
sans le **contenir**, parce que `[.]` est une classe de caractères en regex :

```bash
grep -rnE 'service-public[.]fr' crates/ docs/
```

C'est aussi, en une ligne, la démonstration de R1 : le lint ne distingue pas une
mention d'une prescription, donc un correctif de prose doit retirer le littéral,
jamais l'habiller.

---

## Contrat de vérification

| # | commande | attendu |
|---|---|---|
| V1 | `bash scripts/verify-egress-uniqueness.sh` | exit 0, et la ligne de **contrôle positif** `No egress-uniqueness violations found (5 confined host(s) from docs/egress/egress-manifest.toml).` |
| V2 | `cargo test -p mika-agent --test eval mixed_verification_testimony` | 2 tests passent (`test_testimony_qualified_as_source_not_probative`, `test_regression_testimony_tagged_as_rule`) |
| V3 | `bash scripts/verify-egress-manifest.sh --report` | exit 0 (N1 rejoue V1) |
| V4 | `bash scripts/test-verify-egress-manifest.sh` | exit 0 — le méta-test du lint, non-régression de N16/N17 |
| V5 | `grep -nE 'service-public[.]fr' crates/mika-agent/tests/eval/grounding_regressions/mixed_verification_testimony_as_rule.rs crates/mika-agent/tests/eval/grounding_regressions/fixtures/mixed_verification_testimony_as_rule_pre_fix.json` | **aucune ligne** |
| V6 | `cargo test -p mika-agent --test eval grounding_assertions` | les tests du helper passent — contrôle de non-régression du contrat de tier |
| V7 | `bash scripts/check-canonical-tokens.sh` | exit 0 — ce plan et le plan édité restent conformes |

**V1 porte son propre contrôle de non-vacuité**, et c'est pourquoi la ligne
compte : depuis mika#2408 les motifs sont **dérivés** du manifeste, donc « aucune
violation » a deux causes possibles — l'arbre est propre, ou la liste a rétréci
sans qu'on le voie. Le nombre `5` est la mesure qui les sépare. **Un exit 0 avec
un compte différent de 5 est une halte** : la dérivation a bougé, et ce n'est
alors pas ce correctif qu'on observe.

**V5 nomme deux fichiers et NON le répertoire, et c'est une correction mesurée.**
Un `grep -r` sur `grounding_regressions/` rend **six** hits avant correctif, pas
cinq : le sixième est `mixed_verification_qualification.rs:81`, le frère
mika#1970 **allowlisté et hors périmètre**. Scopée au répertoire, V5 rendrait donc
une ligne même après un correctif parfaitement juste — et un implémenteur qui la
suit à la lettre conclurait à l'échec, ou éditerait le fichier que le § *Ce qu'il
ne faut PAS éditer* lui interdit. C'est aussi, concrètement, le coût du suivi
nommé au § *Hors périmètre* : après ce correctif, ce répertoire porte **deux
conventions**, et un grep nu y devient ambigu.

### Trois haltes

**Halte 1 — V1 reste rouge après les six éditions.** Ne pas élargir le lint et ne
pas ajouter d'entrée d'allowlist par réflexe : relire la sortie, qui **nomme
fichier et ligne**. La cause la plus probable est un septième site apparu depuis
(R3 montre que le ticket en avait déjà omis un), ou une occurrence dans un
fichier non listé au § *Constat*.

**Halte 2 — V2 rouge.** L'analyse de la fenêtre ci-dessus est fausse quelque
part. Lire le message de `panic!`, qui nomme l'élément, le tier déclaré, le
marqueur requis manquant et le marqueur plus fort trouvé. **Ne pas rallonger
`QUALIFICATION_WINDOW_BYTES`** — ce serait changer le contrat d'un helper pour
accommoder un nom d'hôte ; c'est l'hôte qu'il faut raccourcir, ou l'ordre des
éléments qu'il faut relire (le doc-comment prescrit *strongest first*).

**Halte 3 — V2 vert mais `test_regression_testimony_tagged_as_rule` ne refuse
plus.** Ce test assert `result.is_err()` : s'il devenait vert pour la mauvaise
raison, le contrôle négatif serait mort. Lire d'abord V6, qui épingle le contrat
du helper sur des fixtures que ce correctif ne touche pas — si V6 est vert et V2
suspect, la cause est dans le fixture édité, pas dans le helper.

---

## Ce que ce travail n'achète PAS

- **Il n'empêche pas la récidive côté merge.** #2614 a été mergée avec ces deux
  checks **déjà en FAILURE** sur sa tête `8ccaabc8`, parce qu'ils ne sont requis
  par aucune règle de branche. Ce correctif verdit main ; il ne ferme pas la porte
  par laquelle le rouge est entré. C'est le suivi nommé ci-dessous, et c'est la
  moitié la plus utile de l'analyse.
- **Il n'ajoute aucun compteur, aucun événement de journal, aucune ligne
  d'audit.** Le seul instrument est le job CI qui existe déjà, et il est
  bloquant-par-nature : son silence *est* le signal, il n'y a rien à sonder.
- **Il ne retire aucune entrée d'allowlist.** Les deux exemptions de mika#1970
  restent en place et restent permanentes.
- **Il ne rend pas la fixture plus réaliste.** Elle l'est moins : un hôte fictif
  à la place d'un hôte réel. C'est le prix assumé de l'attendu 1, et il est nul
  en pouvoir d'assertion (§ *Ce que le test épingle réellement*).

---

## Hors périmètre, délibérément

- **Les deux jobs egress ne sont requis par aucune règle de branche.** Un lint
  capable de rougir main est un lint qu'aucune garde de merge ne lit — c'est un
  trou de substrat de boucle, et il est de la même classe que ce ticket (tier 1).
  Il se referme par un geste de **réglages de dépôt** (ruleset GitHub), pas par
  une ligne de code, et il n'est pas atteignable depuis un bac à sable de
  dispatch. **Ticket de suivi**, précondition écrite : relever les checks requis
  actuels (`gh api repos/senara-solutions/mika/rulesets`) et les confronter à la
  liste des jobs de `ci.yml`, pour savoir **combien** de lints sont dans ce cas
  plutôt que ces deux-là seulement.
- **Les deux fixtures mika#1970 déjà allowlistées** (5 occurrences,
  `mixed_verification_qualification.rs` + `grounding_assertions/mod.rs`). Les
  convertir au même hôte fictif permettrait de **retirer leurs deux entrées** de
  `AUTHORIZED_PATHS` — ce qui rendrait vraie la phrase que le ticket invoque
  (« cette liste est réservée aux sites qui réalisent ou documentent le
  substrat ») au lieu de la laisser fausse (R2). Hors périmètre ici parce que ces
  fichiers **ne font pas échouer le lint** et qu'élargir un correctif tier 1 à
  une fixture mika#1970 gelée se paie en risque pour un gain d'hygiène.
  **Ticket de suivi.** Coût de ne pas le faire, nommé : deux scénarios frères
  portent désormais deux conventions, et un futur auteur copiera celle du fichier
  qu'il ouvre en premier.
- **Élargir le lint** (ignorer les backticks, les blocs clôturés, ou le
  répertoire `docs/plans/`). Refusé par les attendus 1 et 2 du ticket, deux fois
  et explicitement. Le prédicat est volontairement grossier : il ne distingue pas
  une mention d'une prescription, et c'est ce qui le rend impossible à contourner
  par une mise en forme.
- **La cause racine côté mika#1984**, c'est-à-dire pourquoi un plan groomé par
  deux passes architecte a prescrit un hôte confiné dans une fixture. La réponse
  tient en une ligne — rien dans la chaîne de grooming ne connaît la liste des
  hôtes confinés, et le lint n'est pas exécutable depuis un bac à sable de
  dispatch — mais la refermer demanderait d'ajouter une vérification au
  pré-vol du pilote, ce qui est un changement de substrat de boucle, pas une
  correction de fixture.

---

## Definition of Done

1. Les six sites du § *Constat* ne portent plus le littéral de l'hôte confiné.
2. `bash scripts/verify-egress-uniqueness.sh` sort 0 **avec** sa ligne de
   contrôle positif à 5 hôtes confinés.
3. `bash scripts/verify-egress-manifest.sh --report` sort 0.
4. Les deux tests de `mixed_verification_testimony_as_rule` passent, et les tests
   du helper `grounding_assertions` passent.
5. `AUTHORIZED_PATHS` et `LEGACY_ALLOWLIST` de `verify-egress-uniqueness.sh` sont
   **inchangés** ; aucun motif du lint n'est modifié ; `egress-manifest.toml` est
   inchangé.
6. Aucun fichier de `crates/*/src` n'est modifié.
7. Le corps de la PR nomme R1 (les backticks ne sauvent pas), R2 (la doctrine
   citée est hors contexte, le refus tient sur un autre motif) et R3 (6 sites et
   non 5), et nomme les deux tickets de suivi.

## Acceptance criteria

Transcrits de la section *Attendu* du ticket, avec leur statut d'exécutabilité
après les rectifications ci-dessus.

1. **Les fixtures et le test de `mixed_verification_testimony_as_rule` citent un
   hôte non confiné et manifestement fictif.** Satisfait par
   `demarches-exemple.invalid` sur les quatre sites `.rs` et le site JSON.
   La réserve du ticket — « si un contrôle du test dépend de la forme `.gouv.fr` /
   service public, garder la forme avec un hôte non confiné » — est **sans objet,
   et c'est mesuré** : aucun contrôle ne dépend de la forme du domaine
   (§ *Ce que le test épingle réellement*). La forme est néanmoins conservée
   (un hôte d'allure administrative française), par lisibilité.
2. **Le plan `docs/plans/2026-09-30-002-…` ligne 216 : même traitement.** Satisfait
   par R6. La branche alternative proposée par le ticket — « ou le mettre entre
   backticks si le lint les ignore — à vérifier » — est **vérifiée et réfutée**
   (R1) : le littéral doit disparaître, les backticks n'y changent rien. Le lint
   n'est pas élargi.
3. **Refusé : ajouter ces fichiers à `AUTHORIZED_PATHS`.** Respecté — aucune
   entrée ajoutée, aucune retirée. Le motif du refus est corrigé par R2 : la
   doctrine citée porte sur le builtin `fetch_url`, et la liste contient déjà
   deux entrées de cette classe ; le refus tient sur l'exemption permanente
   qu'une entrée créerait, et sur le critère explicite des trois plans déjà
   allowlistés (documenter le substrat), que le plan mika#1984 ne satisfait pas.
4. **Vérification : `verify-egress-uniqueness.sh` et
   `verify-egress-manifest.sh --report` verts sur la branche, et le test eval
   concerné toujours vert, avec son contrôle négatif rouge.** Satisfait par
   V1/V3 et V2. « Contrôle négatif rouge » se lit : le helper **refuse** toujours
   un témoignage étiqueté `[vérifié: …]` — ce que `test_regression_testimony_tagged_as_rule`
   atteste en asserant `result.is_err()`, et ce que V6 épingle indépendamment du
   fixture édité.
5. **Main redevient verte** et une PR ouverte peut atteindre CLEAN sur ces deux
   jobs — l'objectif de classe tier 1 du ticket.
