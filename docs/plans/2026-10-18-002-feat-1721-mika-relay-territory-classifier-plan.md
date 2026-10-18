# mika#1721 — Classificateur de territoire pour mika-relay : les cinq pièces porteuses ont été mesurées, et aucune n'existe

> **Ticket :** senara-solutions/mika#1721
> Plan content-only produit par `/mika-groom-plan-only`.
> Labels portés : `enhancement`, `p2-normal`, `agent-core`, `dispatch:loop`.
> Parent : senara-solutions/mika#1641 (**fermé**).
> Jumeau : senara-solutions/mika#1720 — « Ticket A », fermé `NOT_PLANNED` le **2026-10-18 à 17:55 UTC**, quelques heures avant ce grooming.

---

**Disposition de ce plan : halte-et-remontée.** Il ne décrit aucune
implémentation, parce que la mesure du § 2 établit que les cinq composants dont
les sept critères d'acceptation dépendent sont absents du dépôt ou hors de son
périmètre — dont l'espace d'adressage de sortie, refusé par Vincent le jour même
de ce grooming. La décision qui reste est une décision de propriétaire, pas
d'architecte : le § 5 la nomme, en donne les deux dispositions candidates et
recommande l'une des deux sans la prendre.

---

## 1. Ce que le ticket demande, et à quoi il l'attache

Le ticket part d'une défaillance observée le 2026-07-03 : orchestrator-CC saute
des barreaux de l'échelle d'escalade — il a posé à Vincent une question de cap
qui relevait de Prime, puis une question sémantique qui relevait de l'architecte.
Le remède proposé est un **classificateur de territoire** greffé sur mika-relay,
qui étiquette chaque question entrante avec une **adresse complète**
`{agent, session}` plutôt qu'un label de portée, écrit l'étiquette dans un
journal d'audit, et ne route rien (livraison « déclawed », recommendation-only).
Il porte un argument d'échappement explicite au précédent mika#1161
(14 % de dérive mesurée sur le motif « LLM en siège de décision ») et un critère
de promotion **pré-enregistré** : ≥ 90 % d'accord global, ≥ 80 % par classe,
≥ 30 échantillons par classe, **zéro** faux positif Prime, ≥ 4 semaines de
trafic.

L'intention est saine et le ticket est rigoureux : il anticipe sa propre
objection, et il pré-enregistre sa barre pour qu'on ne puisse pas la négocier
après coup. Ce qui ne tient pas, c'est **à quoi** il attache tout cela. Cinq
pièces, nommément :

| pièce | rôle dans le ticket | critères qui en dépendent |
|---|---|---|
| l'agent `mika-relay` | l'hôte du classificateur | AC1, AC2, AC3, AC5, AC7 |
| l'espace d'adresses de mika#1720 | le type de sortie | AC2, AC6 |
| le bus `gentux` et sa matrice de créances | l'invariant 2, la garde de livraison | AC4 |
| `control-monitor` (`cm`) | la couche de livraison testée | AC4 |
| le siège orchestrator-CC de juillet | le producteur du trafic à classer | la raison d'être |

Le § 2 les mesure une par une.

---

## 2. Mesures — exécutées au grooming, pas déduites

> **Attestation de re-vérification (passe du 2026-10-18).** Chaque citation de ce
> § a été réexécutée : les cinq états de ticket (`gh issue view` sur #1720, #1641,
> #1719, #1193, #1161), les deux déclarations de `well_known_agents.rs`, la
> migration v39→v40, `CURRENT_SCHEMA_VERSION = 56`, l'absence du répertoire
> `skills/bundled/permission-policy`, l'absence de tout fichier `gentux*`, et le
> grep `publish_allow`. Aucune n'a bougé. Les deux ajouts que cette passe a
> produits — la ventilation des 25 occurrences de `mika-relay` dans `crates/`
> (§ 2.1) et le protocole design-locked du § 2.6 point 3 — vont l'un dans le sens
> du constat, l'autre dans le sens du ticket, et les deux sont dits comme tels.

### 2.1 L'agent hôte n'existe plus : il a été retiré, et pour cette raison-là

`mika-relay` n'est pas « inactif » ou « à recâbler ». Il a été **supprimé**, et
la cause de sa suppression est le précédent même dont le ticket cherche à
s'échapper.

- `crates/mika-agent/src/well_known_agents.rs:567` —
  `pub static WELL_KNOWN_AGENTS: &[&WellKnownAgent] = &[&MIKA_DEV, &MIKA_TEST, &MIKA_QA, &MIKA_ARCH];`
  Quatre entrées. Pas de relais.
- `crates/mika-agent/src/well_known_agents.rs:910` —
  `pub const INTRA_PLATFORM_DISPATCH_PEERS: &[&str] = &["mika-arch", "mika-dev", "mika-qa"];`
  Pas de relais non plus : il n'est même plus une destination joignable.
- `crates/mika-agent/src/db/migrations.rs:3775-3828` — migration v39→v40,
  commentaire `/// v39→v40: Delete all mika-relay agent data (mika#1193).`,
  neuf `DELETE` en ordre de dépendance inverse jusqu'à
  `DELETE FROM agents WHERE id = 'mika-relay';`. Le schéma courant est
  `CURRENT_SCHEMA_VERSION: i64 = 56` (`crates/mika-agent/src/db.rs:35`) : toute
  base vivante a franchi v40, donc la purge a déjà eu lieu partout.
- `skills/bundled/` ne contient plus `permission-policy` — la seule compétence
  que le relais portait.
- `docs/agent-internals/exec-handlers.md:1117` l'écrit sans ambiguïté :
  `MIKA_RELAY_IDENTITY` « exists nowhere in `crates/` and is dropped ».
- Le ticket de retrait : mika#1193, *« retire mika-relay agent +
  permission-policy skill + config refs (mika#1188 Phase C) »*, fermé
  **COMPLETED** le 2026-05-30.

**Le grep que ferait un relecteur rend 25 occurrences, et aucune n'est un chemin
de production.** Il faut le dire ici, parce que `grep -rn mika-relay crates/`
est exactement la vérification qu'on lance pour contredire ce § — et son volume
seul pourrait se lire comme une preuve d'existence. Leur répartition, mesurée :

| site | occ. | nature |
|---|---|---|
| `crates/mika-agent/src/db/migrations.rs` | 12 | **la migration de suppression elle-même** (v39→v40) |
| `crates/mika-agent/src/db/tests/task_messages_and_groom.rs` | 5 | fixture de test — `register_agent("mika-relay", …)`, un id d'agent arbitraire |
| `crates/mika-agent/src/server/budget_guard.rs` | 3 | fixtures sous `#[cfg(test)] mod tests` (`:170`) |
| `crates/mika-cli/src/cli.rs` | 2 | fixtures sous `#[cfg(test)]` (`:1459`) — test d'analyse d'arguments |
| `crates/mika-agent/docs/configuration.md` | 2 | un exemple illustratif (`:270`) et une **confirmation** (`:484`) |
| `crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md` | 1 | doc design-locked — voir § 2.6, point 3 |

Zéro constante d'identité, zéro entrée dans `WELL_KNOWN_AGENTS`, zéro compétence
empaquetée, zéro chemin de code atteignable à l'exécution. Et `configuration.md:484`
dit la conclusion de ce § avec les mots du dépôt : « mika-prime / mika-relay are
**not well-known agents at all**, so they have [no entry] ».

**La cause du retrait est le motif que ce ticket rejoue.**
`docs/solutions/architecture-patterns/deterministic-policy-file-replaces-relay-llm-2026-05-18.md`
documente le remplacement : le tier2 relais-LLM souffrait de « 14% decision
drift (mika#1161) and ~7s per-event latency », remplacé par un fichier de
politique YAML déterministe, avec pour justification *« Unknown events escalate
rather than being decided by an LLM that might hallucinate an allow »*. Le
ticket connaît ce précédent et construit son argument d'échappement contre lui
— mais il suppose que le relais a survécu au précédent en tant que siège. Il n'y
a pas survécu : **la phase C l'a démonté**. Greffer un second domaine de
décision sur `mika-relay` n'est pas « réutiliser un agent existant », c'est le
**ressusciter**, ce que le ticket ne demande nulle part et ne chiffre pas.

**Le résidu documentaire est ce qui rend le titre du ticket encore plausible, et
il tient en trois lignes de `CLAUDE.md`** — pas une :

- `:313` — « `MIKA_RELAY_IDENTITY` (1 skill) — only `permission-policy` ». Les
  deux moitiés sont fausses depuis mika#1193.
- `:272` — `permission-policy/` dans le diagramme de l'arbre `skills/bundled/`.
  Le répertoire n'existe pas (`ls` : *No such file or directory*).
- `:250` — `permission-policy` cité en exemple de compétence engine-coupled.

**Et le dépôt sait déjà que c'est périmé.** `docs/agent-internals/exec-handlers.md:1117`
non seulement constate que `MIKA_RELAY_IDENTITY` « exists nowhere in `crates/`
and is dropped », mais nomme la staleness et la diffère explicitement : « § *Skills
System* still names it (**pre-existing staleness, out of scope**) ». Ces trois
lignes ne sont donc pas une preuve d'existence contredite par ce plan — elles sont
une dette documentaire déjà identifiée et déjà renvoyée, par un document du dépôt,
avant ce grooming. Elles restent hors du périmètre de ce ticket (§ 9).

### 2.2 L'espace d'adresses de sortie a été refusé le jour même de ce grooming

Le ticket déclare, sous *Related* : « Ticket A (session addressability) — this
ticket's classifier output target space is Ticket A's declared addresses ». Et
AC2 exige que la sortie soit une adresse complète, « Enforced by types ».

- **Ticket A est mika#1720**, fermé `stateReason: NOT_PLANNED`,
  `closedAt: 2026-10-18T17:55:07Z`.
- Le commentaire de fermeture, de Vincent via samidarko, le même jour :
  « Fermé comme périmé… Raison d'être disparue : le parent mika#1641 … est fermé
  depuis le 01/07 et l'orchestration est passée à MPC et à mika-manager, pas à la
  session principale de Mika. Le verrou mika#1719 est levé. … **La forme des
  adresses (`org.<org>.app.<agent>.<session>`, UUIDv5) reviendra avec le chantier
  du bus (docs/brainstorms/gentux-bus-design.md), où elle a sa place.** »
- Et il n'a rien livré avant d'être fermé : pas de
  `crates/mika-common/src/session.rs` (le fichier que son corps désignait comme
  site unique de `NS_MIKA`), et zéro occurrence de `NS_MIKA`, `Uuid::new_v5` ou
  d'une section `[sessions]` dans `crates/`.

Donc AC2 n'a **aucun type** à faire respecter, et sa condition de test
(« unit tests reject scope-label-only outputs ») n'a rien contre quoi discriminer.
Le ticket B ne peut pas émettre dans l'espace du ticket A : cet espace a été
explicitement renvoyé au chantier du bus il y a quelques heures.

### 2.3 Le bus n'existe pas dans ce dépôt, et sa matrice de créances non plus

Le ticket appuie son invariant 2 et son AC4 sur le bus : « DELIVERY to Prime is
gated by the existing bearing-circle (Prime's `publish_allow` row per bus design
§4) ». Le mot « existing » est la supposition à vérifier.

- `docs/brainstorms/gentux-bus-design.md` : **absent**. Aucun fichier `gentux*`
  dans l'arbre.
- Le commit cité, `c45a360`, n'est pas une révision de ce dépôt :
  `fatal: ambiguous argument 'c45a360': unknown revision or path not in the working tree`.
- `publish_allow` / `subscribe_allow` : **zéro occurrence**, tous fichiers, tous
  langages, hors `target/` — **sauf ce plan lui-même**, qui cite les deux noms
  ici. Le grep de vérification rend donc les lignes de ce fichier et rien d'autre ;
  en l'excluant (`--exclude-dir=docs`), il rend vide. Je le précise pour que le
  relecteur qui relance la commande ne lise pas ses propres citations comme une
  contradiction.

La référence d'architecture porteuse du ticket est donc **illisible depuis un
worktree de `mika`**, et le mécanisme de garde sur lequel repose la protection du
cercle de Prime n'est pas « existing » : il est à l'état de brouillon, dans un
document que ce dépôt n'a pas.

### 2.4 La couche de livraison que AC4 teste est un autre dépôt, par doctrine

AC4 : « Delivery layer (cm-api or CLI shim) rejects a `mika-prime` destination
not permitted by the current credential matrix, regardless of classifier output.
**Regression test.** »

- `.claude/commands/cm.md:1-11` est un **pointeur**, et le dit en commentaire de
  périmètre : « Ce fichier est un POINTEUR, pas une commande. La commande réelle
  vit dans senara-solutions/control-monitor … **Ne pas y recopier de pipeline :
  deux copies divergent, et c'est celle qu'on ne lit pas qui reste fausse.** »
  (doctrine Vincent, 2026-08-29).
- `docs/design/control-monitor-scope-decision.md:60-66` mesure la même chose :
  « the repository is not checked out in the `mika-platform` workspace (only a
  built artifact is installed, at
  `/usr/local/share/control-monitor/frontend/assets/`) », et en tire la
  conséquence qui s'applique ici mot pour mot : « **No pull request on `mika` can
  change a line of its code** ».

Un test de non-régression sur la couche de livraison de `cm` n'est donc pas
« difficile » depuis ce dépôt : il est **impossible**, et le contourner en
recopiant la logique de livraison côté `mika` est nommément interdit.

**Et l'invariant 3 du ticket ferme le seul hôte qui, lui, existe.** Le substrat
de messagerie réellement en service n'est pas le bus : c'est `cm dispatch` vers
une boîte de réception sur le système de fichiers. `.claude/commands/mpc-inbox.md`
le décrit : « Every `cm dispatch` targeting entity `mpc` writes into the
workspace path below », `/data/workspace/mika-platform/.claude/inbox/` (chemin
non atteignable depuis ce worktree — je le constate, je ne conclus pas qu'il a
disparu). Or l'invariant 3 du ticket dit : « **CM does not consume scope
labels** », et sa justification est que l'inverse « would put intelligence into
cm and violate ADR-0001's wrapper-only invariant ». Le ticket interdit donc
d'héberger le classement là où le trafic passe aujourd'hui. Ce n'est pas une
contradiction du ticket — c'est un invariant écrit pour un monde à deux
composants (relais + bus) appliqué à un monde qui n'en a qu'un.

*Note de probité sur cette citation :* `docs/adr/001-axum-http-server-architecture.md`
est l'ADR-0001 **de ce dépôt** et ne porte aucun « wrapper-only invariant » — son
unique occurrence de « wrapper » désigne `AsyncDatabase` (`:30`). La doctrine
d'enveloppe existe bien dans l'écosystème (mika#1727 : « TUI as thin HTTP client
of mika-spirit — wrapper-doctrine »), mais l'ADR-0001 que le ticket cite est
celui d'un autre dépôt, et n'est pas vérifiable d'ici. Sixième citation qui ne
résout pas.

### 2.5 Le siège a déménagé, et la défaillance qu'il présentait a déjà une garde déterministe

Deux faits, et le second est le plus important pour la valeur du ticket.

**Le siège.** Le parent mika#1641 est fermé, et le commentaire de fermeture de
mika#1720 nomme où l'orchestration est allée : « à MPC et à mika-manager, pas à
la session principale de Mika ». `mika-manager` est réel et dans ce dépôt
(`crates/mika-agent/src/milestone_manager/`), et sa doctrine est l'opposé d'un
saut d'agent par message :
`docs/agent-internals/milestone-manager-phase-1.md:6` — « **LECTURE seule** —
zero dispatch, zero ticket mutation, zero PR merge. **Two** outbound side
effects », les deux déclarées dans `docs/egress/egress-manifest.toml`. Ajouter un
appel LLM par message entrant dans ce périmètre ne serait pas une extension, ce
serait un changement de nature, avec une déclaration d'egress à l'appui.

**La garde.** Le saut de barreau que le ticket décrit n'est plus laissé à la
discipline. `.claude/commands/mika-ask-prime.md:7` :

> « **This is the only legitimate channel for orchestrator-CC operational
> questions.** Direct `AskUserQuestion` calls targeting Vincent are forbidden by
> [[feedback-orchestrator-questions-route-through-prime]] and **structurally
> blocked** by the `AskUserQuestion` PreToolUse hook at
> `.claude/hooks/check-question-routing.sh`. »

C'est exactement la forme de remède que mika#1188 avait choisie contre
mika#1161 : une règle déterministe à la place d'un LLM en siège de décision.
Appliquée, cette fois, au problème de ce ticket-ci. La prémisse de la section
*Description* — « `feedback_operator_is_escalate_only` is a discipline rule that
ends up depending on orchestrator-CC not skipping rungs » — n'est donc plus
vraie : elle dépend aujourd'hui d'un hook, pas d'une discipline.

*Probité :* le script `.claude/hooks/check-question-routing.sh` **n'est pas
présent dans ce worktree** (`.claude/hooks/` n'existe pas ici ; les commandes de
`.claude/commands/` y sont semées par `_seed_worktree_slash_commands`, mika#1415,
et le hook vit dans le checkout de l'orchestrateur). Ce qui est vérifiable d'ici
est le **contrat que l'orchestrateur lit**, cité ci-dessus. Je ne prétends pas
avoir exécuté le hook.

**Et les barreaux eux-mêmes sont hors d'atteinte d'une PR sur `mika`.** Les
quatre rungs de l'échelle — `mika-ask`, `mika-ask-arch`, `mika-ask-prime`,
`mika-ask-a-friend` — sont des fichiers **semés et non suivis** :
`git ls-files .claude/commands/` rend exactement quatre entrées
(`mika-doc-audit.md`, `mika-issue.md`, `mika-issues.md`, `mika.md`), dont aucune
n'est un rung. `CLAUDE.md` l'énonce comme règle : « un ticket opené sur
`senara-solutions/mika` ne peut pas les éditer ».

### 2.6 Ce que la mesure ne retire pas au ticket

Trois choses tiennent, et il faut le dire parce qu'elles pèsent sur la décision
du § 5. La troisième a été trouvée en vérifiant le § 2.1, et elle va dans le sens
du ticket.

1. **Le verrou de séquencement est levé.** mika#1719 est fermé. La *Sequencing
   note* du ticket n'est plus un obstacle — ce n'est simplement plus le seul
   obstacle.
2. **L'argument d'échappement et la barre pré-enregistrée sont du travail
   intellectuel valide et autonome.** L'asymétrie de coût (échec ouvert et
   irrécupérable pour une permission ; échec récupérable par redirection pour un
   routage, sauf la classe Prime, exclue de la moyenne et à zéro dur) ne dépend
   d'aucun des cinq composants mesurés. Elle est vraie, elle est bien argumentée,
   et elle resterait le bon cadre pour n'importe quel classificateur de routage
   futur. C'est la seule part du ticket qui survit à la disparition de son hôte —
   et c'est précisément sur elle que porte la disposition D2 du § 5.
3. **L'invariant 1 du ticket est devenu doctrine du dépôt trois jours après son
   ouverture — et il y est ratifié, pas seulement proposé.** Le ticket a été
   ouvert le 2026-07-03 à 10:15 UTC. Le 2026-07-06 à 17:00 CEST,
   `crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md` est
   **design-locked sur ratification de Vincent** (mika#1733, sous-chantier C de
   mika#1727 — *wrapper-doctrine*, encore OPEN). Sa § *Where this rule is enforced
   structurally* range « any decision produced by the **mika-relay /
   permission-policy skill**, mika-arch verdicts, mika-qa acceptance judgments »
   dans la catégorie **« LLM classifier decisions (NEVER final-at-wire) »**, avec
   la formule : « These have a documented false-deny failure class ; **they
   inform, they do not structurally block.** »

   C'est l'invariant 1 de ce ticket — « Classifier LABELS, does not DELIVER » —
   énoncé comme règle générale du dépôt, séparant les gardes déterministes
   *pre-classifier* (tier1, validation de chemin, denylist d'identité, veto
   structurel par regex) des décisions de classificateur, qui informent. La
   posture « declawed » du ticket n'est donc pas une concession qu'il fait pour
   désarmer l'objection mika#1161 : c'est la seule posture que ce dépôt autorise
   à un classificateur LLM, quel qu'il soit, et elle a été ratifiée
   indépendamment, dans un autre chantier.

   **Ce que cela change, et ce que cela ne change pas.** Cela renforce la clause
   « ce que la fermeture ne perd pas » de D1 (§ 5) : la part doctrinale du ticket
   est déjà tenue ailleurs, par un document design-locked, donc la fermer ne la
   perd pas. Cela ne renverse **pas** la recommandation : la doctrine ratifie la
   *posture*, jamais l'*hôte*, et c'est l'hôte qui manque (§ 2.1). Un ticket dont
   la posture est doctrine et dont l'hôte est supprimé reste un ticket en avance
   sur son substrat.

---

## 3. Pourquoi ce plan ne propose pas de ré-héberger le classificateur

Trois hôtes de substitution étaient concevables. Chacun est écarté par une mesure
du § 2, pas par un jugement de goût.

| hôte candidat | écarté par |
|---|---|
| `cm` / la boîte de réception MPC | l'**invariant 3 du ticket** lui-même (« CM does not consume scope labels »), plus § 2.4 : autre dépôt, recopie de pipeline nommément interdite. |
| `mika-manager` | § 2.5 : périmètre « LECTURE seule », deux effets de bord déclarés ; un appel LLM par message entrant est un changement de nature avec déclaration d'egress, que le ticket ne demande pas. |
| un nouvel agent `mika-relay` reconstruit | § 2.1 : ce serait ressusciter l'agent que mika#1188 phase C a démonté. Le ticket ne le demande pas, ne le chiffre pas, et son argument d'échappement porte sur le *motif*, jamais sur la *reconstruction de l'agent*. |

Choisir l'un des trois serait **réécrire le périmètre du ticket**, pas le
groomer : les sept critères d'acceptation sont écrits pour un hôte nommé, un
espace d'adresses nommé et une couche de livraison nommée. Et le faire
re-litigerait par implication la décision que Vincent a prise il y a quelques
heures sur le jumeau de ce ticket — à savoir que la forme d'adressage « reviendra
avec le chantier du bus, où elle a sa place ».

---

## 4. Definition of Done

**DoD de ce grooming** — atteint par ce document :

1. Les cinq pièces porteuses du ticket sont mesurées une par une, chacune avec
   une citation vérifiable (§ 2.1 à § 2.5).
2. Les sept critères d'acceptation sont transcrits verbatim et chacun reçoit son
   état d'atteignabilité mesuré (§ 6).
3. La décision restante est nommée, attribuée à son propriétaire, et présentée
   avec ses deux dispositions candidates et leurs conséquences (§ 5).
4. Aucun hôte de substitution n'est choisi à la place du propriétaire (§ 3).

**DoD de l'implémentation** — **indéfinie, et c'est le constat central de ce
plan.** Elle ne peut pas être écrite : chacune des sept conditions de sortie du
ticket se lie à un composant absent ou hors périmètre. Il n'y a pas ici une
décision de conception qu'un architecte pourrait trancher par citation de
principe ; il y a une décision de propriétaire sur le **sort du ticket**. C'est
la branche 2 de l'*Unresolved-Decision Gate* : « unresolved decisions AND they
genuinely require operator judgment outside architect authority ».

---

## 5. La décision à remonter, et sa recommandation

**La décision, en une phrase :** mika#1721 doit-il être fermé comme périmé — même
disposition et même cause que son jumeau mika#1720, fermé il y a quelques heures
— ou son périmètre doit-il être réduit à la seule part qui survit, le
pré-enregistrement ?

Elle appartient à Vincent. Elle n'appartient pas à l'architecte : trancher
suppose de dire si un chantier reste voulu, pas de comparer deux conceptions.

### D1 — Fermer mika#1721 comme périmé *(recommandée)*

Le motif est celui de mika#1720, et il s'applique terme pour terme : parent fermé,
siège déménagé, substrat d'adressage renvoyé au chantier du bus. S'y ajoute une
cause que le jumeau n'avait pas — l'hôte du livrable a été **supprimé** par
mika#1193, et supprimé pour le précédent même dont ce ticket s'échappe.

**Pourquoi je la recommande.** Les cinq composants sont absents ; sept critères
sur sept sont inatteignables ; et la défaillance que le classificateur devait
corriger a entre-temps reçu une garde déterministe (§ 2.5), ce qui érode la
proposition de valeur indépendamment du problème d'hébergement. Le ticket n'est
pas *faux* — il est **en avance sur son substrat**, et son substrat a été
explicitement reprogrammé pour le chantier du bus.

**Ce que la fermeture ne perd pas, et comment le garantir à bas prix.** Le corps
du ticket reste lisible après fermeture : l'argument d'asymétrie de coût et la
table de promotion y demeurent intacts. Et sa part la plus structurante —
l'invariant 1, « classifier LABELS, does not DELIVER » — ne dépend plus du tout de
ce ticket : elle est portée par un document design-locked et ratifié
(§ 2.6, point 3), donc la fermeture ne peut pas la perdre. Si l'inquiétude est qu'un ticket d'ère-bus
rouvre la question à une barre plus basse, le remède est le même que celui que
mika#1720 a employé pour préserver la forme d'adressage : **un commentaire de
fermeture qui nomme où la chose revient**, et qui désigne mika#1721 comme le
pré-enregistrement de référence que tout classificateur de routage futur doit
satisfaire avant d'être promu. Coût : un commentaire. C'est la raison pour
laquelle D1 n'exige pas D2 pour être sûre.

### D2 — Réduire le périmètre au seul pré-enregistrement

Ne garder que ce qui ne dépend d'aucun des cinq composants : un ADR sous
`docs/adr/` portant (a) l'argument d'échappement à mika#1161 dans sa forme
d'asymétrie de coût, et (b) les cinq seuils de promotion, avec la règle que les
modifier exige un pré-enregistrement amendé et non un argument de revue de
corpus. Le code est renvoyé à un ticket d'ère-bus.

**Son attrait, honnêtement :** c'est l'argument du ticket lui-même — la barre vaut
parce qu'elle est enregistrée *avant* que le premier label soit écrit, et un ADR
survit mieux qu'un corps de ticket fermé.

**Ce qu'elle coûte, et pourquoi je ne la recommande pas en premier :** elle ne
satisfait **aucun** des sept critères à la lettre (AC1 exige le commentaire
« adjacent to the classifier implementation », AC5 des « constants in the
classifier code », AC7 un drapeau de configuration — tous présupposent le
classificateur). Elle réduit donc sept critères à deux-en-esprit, ce qui est un
arbitrage de propriétaire. Et pré-enregistrer par ADR la barre d'un système dont
personne n'a décidé la construction est **en soi** un acte de cap : D2 ne
contourne pas la décision de Vincent, elle en demande une seconde.

### Ce que ce plan ne fait pas

Il ne choisit pas entre D1 et D2, et il n'écrit ni l'ADR de D2 ni le commentaire
de fermeture de D1. Les deux sont des actes de cap. Un plan de grooming qui les
exécuterait sur sa propre autorité fabriquerait la décision qu'il est censé
remonter.

---

## 6. Acceptance criteria

Transcription **verbatim** du corps de mika#1721 (règle mika#1600/mika#1627),
chacune suivie de son état d'atteignabilité mesuré. Les jetons `TBD` qui
apparaissent dans AC3 et dans la table de promotion sont ceux **du ticket**,
conservés par la règle de transcription verbatim ; ils ne sont pas des réserves
de ce plan, et ils sont sans objet sous la mesure du § 2.

- **AC1** — « Escape-argument paragraph (cost-asymmetry) appears as a
  documentation comment adjacent to the classifier implementation. Reviewer can
  locate it via grep for `mika#1161`. »
  → **Inatteignable.** Aucune « classifier implementation » à côté de laquelle
  placer le commentaire : l'hôte est supprimé (§ 2.1).
- **AC2** — « Classifier output is a full address `{agent, session}`, not a scope
  label. Enforced by types; unit tests reject scope-label-only outputs. »
  → **Inatteignable.** Le type d'adresse a été refusé le jour de ce grooming
  (§ 2.2) ; rien à faire respecter, rien contre quoi discriminer.
- **AC3** — « Classifier does not deliver; every classification is written to the
  audit log with (input, output, timestamp, eventually_correct_destination
  TBD-until-adjudicated). »
  → **Inatteignable.** Dépend de l'hôte (§ 2.1) et du type de sortie (§ 2.2).
- **AC4** — « Delivery layer (cm-api or CLI shim) rejects a `mika-prime`
  destination not permitted by the current credential matrix, regardless of
  classifier output. Regression test. »
  → **Inatteignable, et interdit.** La matrice de créances n'existe pas (zéro
  `publish_allow`, § 2.3) et la couche de livraison est un autre dépôt dont la
  recopie est nommément proscrite (§ 2.4).
- **AC5** — « Pre-registered promotion criterion appears in the plan doc AND as
  constants in the classifier code (e.g. `PROMOTION_OVERALL_MIN = 0.90`,
  `PROMOTION_PER_CLASS_MIN = 0.80`, `PROMOTION_MIN_N_PER_CLASS = 30`,
  `PROMOTION_PRIME_FP_MAX = 0`, `PROMOTION_WINDOW_WEEKS = 4`). Editing these
  constants requires an amended pre-registration ADR/plan. »
  → **Première moitié atteinte** : les cinq seuils sont dans ce document (§ 1).
  **Seconde moitié inatteignable** : il n'y a pas de « classifier code » où
  poser les constantes (§ 2.1). C'est le seul critère dont une moitié survit, et
  c'est l'objet de la disposition D2.
- **AC6** — « Corpus audit-log schema records eventually-correct-destination as a
  separate field, adjudicated by orchestrator-CC (or its successor) at the time
  the corrected destination is known. »
  → **Inatteignable.** Le champ est typé par l'espace d'adresses refusé (§ 2.2),
  et l'arbitre nommé — orchestrator-CC — a changé de siège (§ 2.5).
- **AC7** — « Feature flag / config toggle: classifier can be enabled/disabled
  without redeploy. Default OFF at ship. »
  → **Sans objet.** Un drapeau n'a de sens qu'adossé à un chemin de code ; il
  n'y en a aucun.

**Bilan : 0 des 7 critères atteignables, 1 à moitié.** C'est ce bilan, et non une
préférence de conception, qui produit la disposition halte-et-remontée.

**Critères d'acceptation de ce plan en tant que livrable de grooming** — ils sont
les quatre points du DoD de grooming (§ 4) et sont vérifiables par relecture :
chaque pièce mesurée porte une citation `fichier:ligne` ou un état de ticket
horodaté ; aucune disposition n'est choisie à la place du propriétaire.

---

## 7. Taille estimée

Ce plan ne livre aucun code. Les deux dispositions candidates du § 5 ne
produisent, elles non plus, aucune ligne hors `docs/` : D1 est une fermeture de
ticket plus un commentaire, D2 est un fichier sous `docs/adr/` — exclu du compte
par la règle elle-même (« hors `docs/` »).

| livrable | lignes de code (hors `docs/`) |
|---|---|
| ce plan (`docs/plans/2026-10-18-002-…-plan.md`) — exclu du compte, `docs/` | 0 |
| D1 — fermeture de mika#1721 + commentaire de clôture | 0 |
| D2 — ADR de pré-enregistrement sous `docs/adr/` — exclu du compte, `docs/` | 0 |

Total estimé : 0 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi ce plan tient, et en un seul vol.** 0 < 279 et 0 < 1000, sans marge à
discuter : il n'y a pas de découpage à faire, parce qu'il n'y a pas de volume. Le
risque que la règle mika#2636 et la borne mika#2693 adressent — un vol qui meurt
au plafond de tours avant la fin de pipeline — est structurellement absent ici.
Aucune ligne `Tracked in:` n'est due : le travail n'est pas reporté à un vol 2, il
est **remonté** au propriétaire, et le § 2.2 nomme déjà le chantier où la forme
d'adressage revient si la réponse est D1.

**Fire-Disposition : sans objet.** Ce plan ne livre aucun détecteur — ni test, ni
assertion, ni règle de lint, ni garde CI, ni validateur. La section n'est donc pas
requise (gate N/A, mika#2306) et n'est pas inventée.

---

## 8. Risques et arbitrages

**Le risque de se tromper sur la fermeture.** Si Vincent veut le classificateur,
D1 coûte la réouverture d'un ticket — bon marché, et le corps est préservé. Si
l'on avait au contraire planifié une implémentation, le coût eût été un vol de
pilote entier contre cinq composants absents, mort non pas au plafond de tours
mais au premier `cargo build` sur un `mika-relay` inexistant. L'asymétrie est
nette et elle pointe vers la remontée.

**Le risque de redondance avec mika#1720.** Faible et assumé : ce plan ne
re-litige pas la fermeture du jumeau, il mesure ce qu'elle implique pour celui-ci.
Les deux tickets partagent un parent fermé et un substrat reporté ; que la même
cause produise la même disposition est une cohérence, pas une copie.

**Le risque d'avoir mal mesuré.** Quatre constats de ce plan reposent sur des
artefacts **non atteignables depuis ce worktree** et sont étiquetés comme tels
plutôt que supposés : `docs/brainstorms/gentux-bus-design.md` et le commit
`c45a360` (§ 2.3), le dépôt `control-monitor` (§ 2.4), la boîte de réception MPC
et le hook `check-question-routing.sh` (§ 2.5), et l'ADR-0001 « wrapper-only »
cité par le ticket (§ 2.4, note). Aucun des quatre n'est le pivot de la
conclusion : le pivot est l'absence de l'hôte, qui se lit entièrement dans
`crates/` (§ 2.1), et le refus du jumeau, qui se lit dans l'état du ticket
mika#1720 (§ 2.2). Les quatre, s'ils étaient inspectés, ne pourraient que
renforcer le constat.

---

## 9. Hors périmètre, délibérément

- **Corriger les trois lignes périmées de `CLAUDE.md`** (`:250`, `:272`, `:313` —
  `permission-policy` et `MIKA_RELAY_IDENTITY`, périmées depuis mika#1193, § 2.1).
  Réel, trivial, étranger à ce ticket — et **déjà renvoyé** par
  `docs/agent-internals/exec-handlers.md:1117` (« pre-existing staleness, out of
  scope »). À ouvrir séparément ; ce plan ne le fait pas, pour ne pas mêler une
  correction de documentation à une remontée de cap.
- **Reconstruire `mika-relay`.** § 3.
- **Écrire l'ADR de D2 ou le commentaire de clôture de D1.** § 5, dernier
  paragraphe : ce sont des actes de cap.
- **Le chantier du bus `gentux`** et la forme d'adressage `org.<org>.app.<agent>.<session>`
  — renvoyés là par la fermeture de mika#1720, et c'est leur place.

---

## 10. Référence

**Tickets.** mika#1721 (celui-ci, OPEN, ouvert 2026-07-03T10:15:16Z) · mika#1720
Ticket A (CLOSED NOT_PLANNED, 2026-10-18T17:55:07Z) · mika#1641 parent (CLOSED
COMPLETED, 2026-07-01) · mika#1719 verrou de séquencement (CLOSED COMPLETED,
2026-09-30 — levé) · mika#1161 dérive de 14 % (CLOSED, 2026-05-18) · mika#1188 et
mika#1193 (phases du retrait du relais ; #1193 CLOSED COMPLETED 2026-05-30) ·
mika#1727 (doctrine d'enveloppe, **OPEN**) · mika#1733 (sous-chantier C de #1727,
CLOSED 2026-07-13 — le protocole design-locked du § 2.6) · mika#1415 (semis des
commandes de worktree).

**Code.** `crates/mika-agent/src/well_known_agents.rs:567` et `:910` ·
`crates/mika-agent/src/db/migrations.rs:3775-3828` ·
`crates/mika-agent/src/db.rs:35` · `crates/mika-agent/src/milestone_manager/`.
Fixtures de test portant la chaîne `mika-relay`, nommées pour qu'un grep ne les
lise pas comme des chemins de production (§ 2.1) :
`crates/mika-agent/src/server/budget_guard.rs:170` (`#[cfg(test)]`) ·
`crates/mika-cli/src/cli.rs:1459` (`#[cfg(test)]`) ·
`crates/mika-agent/src/db/tests/task_messages_and_groom.rs:959`.

**Documents.** `docs/solutions/architecture-patterns/deterministic-policy-file-replaces-relay-llm-2026-05-18.md` ·
`crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md:96` (design-locked,
§ 2.6 point 3) · `crates/mika-agent/docs/configuration.md:484` et `docs/configuration.md:484` ·
`docs/agent-internals/exec-handlers.md:1117` ·
`docs/agent-internals/milestone-manager-phase-1.md:6` ·
`docs/design/control-monitor-scope-decision.md:60-66` ·
`docs/adr/001-axum-http-server-architecture.md:30` · `CLAUDE.md:250`, `:272`, `:313`.

**Commandes semées (non suivies par ce dépôt).**
`.claude/commands/mika-ask-prime.md:7` · `.claude/commands/cm.md:1-11` ·
`.claude/commands/mpc-inbox.md`.
