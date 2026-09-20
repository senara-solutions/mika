# Plan — mika#2281 : Mika consomme le serveur MCP « 1Password Environments »

- **Ticket :** senara-solutions/mika#2281
- **Type :** feat (capacité produit — client MCP, frontière de confiance des secrets)
- **Date :** 2026-09-20
- **Branche :** `feat/2281/agent-mcp-mika-consomme-le-serveur-mcp`

---

## Goal Capsule

Le ticket demande qu'un agent Mika puisse lister ses Environments 1Password et
monter un `.env`, sans qu'aucune valeur de secret n'atteigne le contexte LLM,
les logs ou les transcripts.

**Ce que ce plan rectifie du ticket, et c'est le premier livrable.** Quatre
affirmations structurantes du ticket sont réfutées par la mesure, et deux d'entre
elles déplacent le travail entier.

### R1 — Le binaire n'est pas installable « sur gentux » : il est *absent*, et il n'est pas distribué seul

Le ticket écrit « Binaire `1password-mcp` (macOS/Linux) » et demande en AC4
« comment installer le binaire sur gentux (Gentoo) ». Mesuré sur gentux le
2026-09-20 :

| Fait | Preuve |
|---|---|
| Le binaire s'appelle `onepassword-mcp`, pas `1password-mcp` | doc éditeur ; chemin Linux `/opt/1Password/onepassword-mcp` |
| Il n'est **pas** distribué séparément — il est **bundlé dans l'app desktop** | doc éditeur (macOS : `/Applications/1Password.app/Contents/MacOS/onepassword-mcp`) |
| `/opt/1Password/` sur gentux **ne le contient pas** | `ls /opt/1Password/` — 28 entrées, aucune ne correspond |
| L'installation locale date du **20 juillet 2023** et est **hors Portage** | `ls -ld /opt/1Password` ; `/var/db/pkg/app-admin/` ne contient aucun paquet 1Password ; fichiers owned par `samidarko` |
| Cette version **précède la fonctionnalité** | `grep -i mcp /opt/1Password/after-install.sh` → aucune occurrence, alors que le script de l'éditeur est précisément l'endroit où le groupe `onepassword-mcp` et le `setgid` sont posés |
| La CLI `op` n'est pas installée non plus | `which op` → not found |

Il n'y a donc **rien à installer au sens d'AC4** : il y a une **app desktop de
trois ans de retard, posée à la main hors du gestionnaire de paquets, à mettre à
jour**. C'est un geste d'opérateur sur la machine, pas une ligne de code, et
c'est la première précondition de tout le reste.

### R2 — Le gate humain n'est pas seulement une contrainte : c'est un mécanisme d'authentification du client, et il rejette probablement Mika

Le ticket traite l'approbation dans l'app (AC3) comme une propriété à *préserver*.
C'est aussi, et d'abord, une **porte d'entrée que Mika doit franchir**, et le
mécanisme est documenté :

- `/opt/1Password/after-install.sh` (versions récentes) crée un groupe
  `onepassword-mcp`, l'assigne au binaire et applique `chmod g+s` pour permettre
  la vérification du pair via `SO_PEERCRED`.
- Le serveur vérifie en outre le **process parent**. Échecs observés en
  production sur Linux : `parent process verification failed: Binary Permissions`
  et `Rejecting MCP connection from pid …: parent process verification failed`
  (journal éditeur : `op-mcp-relay/src/handler/process.rs:141`).
- Diagnostic communautaire convergent : **un parent qui n'est pas owned par root
  est rejeté** — ce qui exclut Claude Code, « presque toujours installé dans
  `/home/[user]/.local/bin` ».

Or `Makefile:1` pose `INSTALL_DIR ?= $(HOME)/.local/bin`. **mika-spirit est
déployé exactement là où le rejet est documenté**, et pour la même raison. Ce
n'est donc pas un risque spéculatif : c'est un échec attendu par construction,
dont la cause est **chez un tiers** et qu'aucune ligne de ce dépôt ne corrige.
Statut au 2026-09-20 : escaladé par le support éditeur, **non résolu**, dernière
réponse de leur équipe fin juillet 2026, contournement par script wrapper
explicitement donné comme non garanti.

**Conséquence sur la forme du plan.** Écrire l'intégration avant de savoir si le
serveur accepte une connexion de Mika, c'est écrire du code dont la précondition
est chez quelqu'un d'autre. Le plan commence donc par une **sonde**, pas par du
code (UI-1), avec un critère de halte écrit.

### R3 — La configuration MCP n'est pas per-agent, et l'isolation exigée par l'opérateur n'est réalisable ni par fichier, ni par denylist

Le ticket dit « configuration `mcp_servers` par agent ». C'était vrai ; ça ne
l'est plus depuis mika#1737, dont la disposition ratifiée est écrite en tête du
résolveur (`crates/mika-common/src/mcp_config_path.rs:4-6`) :

> *Ratified architectural disposition: MCP config is CLI/operator-side
> (edit-time), **not per-agent** or spirit-runtime.*

Le per-agent est explicitement *legacy* (`legacy_per_agent_mcp_path`, `:96-98`)
et migré one-shot vers le chemin global. Le ticket a hérité son erreur de la
documentation, qui n'a pas suivi — voir R4.

Cela heurte frontalement la consigne de Vincent, répétée deux fois dans les
commentaires : **« test par mika-secretary avec l'agent mika-test — JAMAIS en
prod »**. Un serveur ajouté à `~/.config/mika/mcp-servers.json` est lu par
`init_agent` pour **chaque** agent (`server/mod.rs:603`), mika de production
inclus.

Et le repli naturel — restreindre par `identity.toml` — **n'existe pas** :

1. Aucun allowlist/denylist per-agent des serveurs ni des outils MCP.
2. Pire, les outils MCP **contournent la denylist existante** : les définitions
   sont ajoutées en `agent_loop/mod.rs:4513-4515` par un `extend_from_slice` brut,
   alors qu'`apply_agent_tool_visibility(&mut tool_defs, disabled_tools)` s'exécute
   **avant**, à l'intérieur d'`inject_skills_and_resolve_tools` (`:7664`, appelé
   `:4450-4459`). Un `mcp__1password__*` **ne peut pas** être refusé à un agent
   par `[tools].disabled` en l'état.
3. Le même ordre fait que les outils MCP échappent aussi au gate compact-provider
   (`:7668-7675` retourne avant l'append).

**Conséquence.** L'isolation demandée n'est pas une option de configuration : en
l'état, elle ne s'obtient que par **procédure** — ne jamais écrire le serveur dans
le fichier global, et faire la démo dans un process jetable via `MIKA_MCP_CONFIG`
(UI-2). Le scoping per-agent est un ticket à part, qui contredirait une
disposition ratifiée et ne se décide pas ici.

### R4 — AC2 vise le mauvais canal : 1Password garantit déjà le canal MCP ; le chemin de fuite est le `.env` monté

AC2 demande qu'aucune valeur n'apparaisse dans le contexte, les logs et les
transcripts. Sur le **canal MCP**, cette propriété est tenue par le fournisseur,
pas par Mika : *« the server cannot return secret values to the client, even if
an agent requests them »*. Il n'y a rien à implémenter pour ça — seulement à le
vérifier.

Le trou est ailleurs, et le ticket ne le nomme pas. `create_local_env_file`
**monte un `.env`** (FIFO en mémoire, variables injectées dans le process
autorisé). Ce que 1Password garantit, c'est que le *serveur MCP* ne rend pas les
valeurs — **pas** que le fichier monté soit illisible. Or :

- AC1 demande explicitement de monter un `.env` ;
- les agents portent `file-reader` et `shell-exec` dans les allowlists usuelles ;
- `secret_scrubber.rs:5-6` est explicite : *« The LLM's in-memory tool output is
  **NOT** scrubbed — only the durable copy is sanitized »* ;
- et la copie durable elle-même n'est protégée que sur **14 formes connues**
  (`SECRET_PATTERNS`, `:40-80`) — un mot de passe arbitraire ou une chaîne de
  connexion est persisté en clair dans `tool_calls.output`.

**AC1 et AC2 sont donc en tension directe** : monter un `.env` à portée d'un
agent qui sait lire des fichiers, c'est mettre les valeurs à un `read_file` du
contexte. Le test négatif d'AC2 doit couvrir **ce** chemin, pas seulement le
canal MCP où il ne peut rien trouver.

### Ce que le plan livre

Une **sonde de faisabilité** qui tranche R1/R2 avant tout code ; l'**isolation
par procédure** qui rend une démo conforme à la consigne opérateur ; le **test
négatif de non-fuite** sur le chemin réellement exposé, qui a de la valeur pour
tout serveur MCP et survit à un abandon de 1Password ; le **branchement**, s'il
est possible ; et la **documentation** — livrable dans les deux branches du
verdict, y compris s'il est négatif.

---

## Product Contract

**Ce que l'opérateur obtient si la sonde passe.** Une procédure écrite pour
brancher le serveur 1Password Environments sur un process `mika` jetable, une
démonstration `list_environments` + montage d'un `.env` sur un Environment de
test, et la garantie mesurée qu'aucune valeur n'a traversé.

**Ce que l'opérateur obtient si la sonde échoue.** Un constat daté, chiffré et
reproductible de *pourquoi* ce chemin ne tient pas — le binaire absent, le rejet
du parent, ou les deux — avec la condition de réveil. C'est un livrable, pas un
échec : la question « Mika peut-elle utiliser le MCP 1Password ? » reçoit une
réponse documentée au lieu de rester ouverte et de revenir.

**Ce que l'opérateur obtient dans les deux cas.** Le test négatif de non-fuite,
qui protège tout serveur MCP présent et futur ; la correction des quatre dérives
documentaires qui ont produit l'erreur du ticket ; et l'inventaire écrit des
trous du substrat MCP découverts en chemin (isolation, contournement de denylist,
absence de surface opérateur), chacun rattaché à un ticket de suivi plutôt
qu'absorbé en silence.

**Population visée.** L'opérateur sur gentux, en session interactive. Les tours
silencieux (heartbeat, reflection, callback, reminder), les runs d'équipe et les
délégations sont **hors périmètre par construction, pas par décision** : ils
passent `mcp_manager: None` (`agent_loop/mod.rs:5942` — *« MCP tools excluded from
silent mode »* ; `teams/engine.rs:1380`, `:1834` ; `tools/delegate_task.rs:309`).
La réserve d'AC4 — « les crons sans session interactive ne sont pas couverts » —
est donc **déjà tenue par le substrat**, et le plan se contente de l'écrire.

---

## Planning Contract

### Décision centrale — sonder avant d'écrire

Le coût d'une sonde est de quelques dizaines de minutes d'opérateur. Le coût de
son absence est un chantier d'intégration dont la précondition est chez un tiers
qui n'a pas livré de correctif, et qui se solderait par du code mort. La maison
emploie déjà cette forme (mika#2331 : *« Étape 0 — avant toute ligne de code, lire
un instrument qui existe déjà »* ; mika#2280 : *« rien n'est corrigé ici […] ce
travail produit la mesure qui décidera du remède »*).

**La sonde a un critère de halte écrit**, et l'échec est un résultat : il clôt le
ticket sur un constat au lieu de le laisser revenir tous les trimestres.

### Trois refus, chacun avec sa raison

**Refusé — ajouter un scoping per-agent des serveurs MCP pour satisfaire
« JAMAIS en prod ».** Ce serait revenir sur la disposition ratifiée de mika#1737
(session arch `0e01b314`), et il faudrait le faire **deux fois** : un filtre par
serveur *et* un déplacement de l'append MCP derrière `apply_agent_tool_visibility`,
dont le blast radius touche l'ordre d'assemblage du schéma d'outils de chaque
tour. C'est un ticket de substrat à part entière, pour une démo qui n'a pas encore
prouvé qu'elle pouvait se connecter. La consigne opérateur est satisfaite ici par
`MIKA_MCP_CONFIG` sur un process jetable — un levier qui **existe déjà** et dont
le doc-comment dit qu'il est *« load-bearing for tests and non-XDG hosts »*
(`mcp_config_path.rs:26-27`).

**Refusé — scrubber les sorties d'outils MCP vers le LLM.** Ce serait inverser
une doctrine écrite (`secret_scrubber.rs:5-6`) pour tous les outils, pas seulement
MCP, et sur 14 motifs qui ne couvrent de toute façon pas un secret arbitraire. Le
remède au vrai risque (R4) n'est pas de filtrer ce qui revient — le serveur
1Password ne renvoie rien — mais de **ne pas monter un `.env` à portée d'un agent
qui lit des fichiers**. C'est une contrainte de procédure et un test, pas un
filtre.

**Refusé — un serveur MCP communautaire adossé à `op` + service account comme
plan B silencieux.** Ces serveurs (`op-mcp`, `pulsemcp/onepassword`) fonctionnent
en headless, ce qui les rend séduisants pour les crons — et ils **inversent
exactement la propriété que le ticket achète** : les valeurs transitent en clair
par le canal MCP et vers le modèle. Basculer dessus au motif que l'officiel
résiste, ce serait satisfaire AC1 en détruisant AC2 et AC3. Si le chemin
service-account est voulu, c'est la « autre décision » que le ticket nomme déjà
lui-même en AC4, et elle se prend séparément.

### Ce que le plan ne peut pas garantir, et le dit

**AC1 n'est pas garantissable par ce plan.** Sa précondition — que le serveur
1Password accepte une connexion dont le parent est mika-spirit ou `mika` — est
détenue par un tiers, et le défaut documenté est ouvert. Le plan garantit que la
question sera **tranchée par la mesure et écrite**, pas qu'elle sera tranchée
favorablement. Toute formulation plus forte serait une promesse sur le calendrier
de quelqu'un d'autre.

---

## Implementation Units

### UI-1 — Sonde de faisabilité (aucun code, verdict écrit)

Trois questions séquentielles, chacune avec sa halte. Rien n'est écrit dans
`~/.config/mika/mcp-servers.json` : la sonde utilise un fichier jetable pointé par
`MIKA_MCP_CONFIG` (voir UI-2).

**Q1 — le binaire existe-t-il ?** Après mise à jour de l'app desktop 1Password
sur gentux (geste opérateur, hors Portage, voir R1) :
`ls -l /opt/1Password/onepassword-mcp`.
*Halte :* absent ⇒ verdict `BLOQUÉ-AMONT`, la fonctionnalité n'est pas livrée
dans le paquet Linux de cette version. Aller à UI-5 branche négative. Ne pas
chercher de binaire ailleurs, ne pas basculer sur un serveur communautaire (refus
ci-dessus).

**Q2 — le serveur accepte-t-il un client Mika ?** C'est la question du ticket, et
elle se répond sans écrire une ligne de Rust — le substrat MCP est générique.
Activer la fonctionnalité côté app (Settings → Labs → *Enable local MCP server*,
puis Settings → Developer → *Integrate with MCP clients*), puis lancer un
`mika chat` jetable avec `MIKA_MCP_CONFIG` pointant sur une config à un serveur
stdio `/opt/1Password/onepassword-mcp`.
Lecture : `grep -E 'connected to MCP server|failed to connect to MCP server'` sur
la sortie du process.
*Halte :* un rejet portant `parent process verification` ou `peer effective GID`
⇒ verdict `BLOQUÉ-TIERS`. **Ne pas tenter le script wrapper en production.** Il
est donné comme non garanti par l'éditeur, et son effet est de maquiller le
parent pour franchir un contrôle d'authentification — c'est-à-dire de contourner
la vérification d'identité du client qui est précisément ce qui donne sa valeur à
AC3. Le mentionner dans le constat comme piste amont, pas comme remède.
*Note de lecture :* `connect_all` est fail-open (`mcp/mod.rs:53-57`) — un serveur
qui échoue est simplement absent, sans message d'erreur au niveau de l'agent. Le
`warn!` du journal est le seul signal ; son absence ne vaut pas succès.
*Second effet à surveiller :* `connect_all` est `await`-é dans `init_agent`
(`server/mod.rs:607`) avec un handshake borné à 30 s (`mcp/mod.rs:264-270`). Un
serveur qui attend une approbation GUI peut donc **ajouter jusqu'à 30 s au
démarrage** de tout process qui le charge — raison de plus pour ne jamais poser ce
serveur dans le fichier global lu par le démon.

**Q3 — l'approbation est-elle atteignable depuis le process ?** Si Q2 passe, un
prompt doit s'afficher dans l'app. L'allowlist d'environnement des enfants stdio
(`mcp/mod.rs:234-246`) transmet `XDG_RUNTIME_DIR` — ce qui est la bonne nouvelle —
mais **ni `DBUS_SESSION_BUS_ADDRESS`, ni `WAYLAND_DISPLAY`, ni `DISPLAY`**.
*Halte :* handshake accepté mais aucun prompt ⇒ consigner quelle variable manque.
L'élargissement de l'allowlist est un changement de posture de sécurité sur
**tous** les serveurs MCP : le noter, ne pas le faire dans ce ticket.

**Livrable :** un verdict écrit parmi `OK` / `BLOQUÉ-AMONT` / `BLOQUÉ-TIERS` /
`BLOQUÉ-SESSION`, avec les sorties brutes, consigné dans le document d'UI-5.

### UI-2 — Isolation par procédure (précondition de toute démo)

Aucune ligne de production. La démo se fait dans un process `mika` jetable dont
`MIKA_MCP_CONFIG` pointe un fichier temporaire ; le fichier global reste vierge.
`resolve_operator_mcp_config_path` donne la priorité 1 à cette variable
(`mcp_config_path.rs:61-65`), donc le process de démo voit le serveur et **aucun
autre process ne le voit**.

Ce que la procédure doit écrire noir sur blanc, parce que c'est contre-intuitif
et que le ticket s'y est déjà trompé :

1. Écrire le serveur dans `~/.config/mika/mcp-servers.json` l'expose à **tous**
   les agents, mika de production inclus (R3).
2. `[tools].disabled` d'`identity.toml` **ne peut pas** le refuser à un agent, les
   outils MCP étant ajoutés après le filtre de visibilité (R3).
3. Donc, tant que le scoping per-agent n'existe pas, *« tester avec mika-test,
   jamais en prod »* signifie **process jetable**, et non *agent différent*.

Point d'ergonomie à consigner : `mika mcp add` n'expose pas de `--env`
(`cli.rs:1156-1175`, `commands/mcp.rs:146` code en dur `env: None`). Toute
variable nécessaire au serveur impose d'éditer le JSON à la main. À nommer dans la
doc ; l'ajout du drapeau est un ticket de suivi.

### UI-3 — Test négatif de non-fuite (AC2), indépendant de 1Password

C'est le travail de code du ticket, et il a de la valeur même si le verdict d'UI-1
est négatif : il porte sur le **chemin MCP générique**, donc sur tout serveur
présent et futur.

Le substrat manque de ce qu'il faut pour le faire : il n'existe **aucun serveur
MCP factice, aucun harnais, aucune fixture de protocole**, et
`EvalHarness::mcp_manager()` (`tests/eval/harness.rs:469-472`) n'a **aucun
appelant** — son seul constructeur public, `connect_all`, exige un vrai serveur.

Construire donc un serveur MCP factice minimal — un binaire stdio de fixture
répondant `initialize` / `tools/list` / `tools/call` — exposant un outil qui rend
une valeur sentinelle connue (une chaîne qui n'appartient à **aucun** des 14
motifs de `SECRET_PATTERNS`, pour mesurer la couverture réelle et non celle du
scrubber). Puis asserter, sur un tour d'agent complet :

1. la valeur sentinelle **n'apparaît pas** dans `tool_calls.output` ni
   `tool_calls.input` après scrubbing ;
2. elle n'apparaît pas dans `messages.metadata` (`ToolCallSummary`) ;
3. elle n'apparaît pas dans le journal, **y compris avec `MIKA_LOG_LLM_BODIES`
   armé** — c'est la seule configuration où les corps de requête sont écrits, donc
   la seule où AC2 est réellement testé ;
4. **le contrôle positif** : la même sentinelle traversant le canal « fichier »
   (un `.env` monté puis lu par un outil de lecture de fichier) **doit** être
   trouvée. Sans ce contrôle, un test tout-vert n'établit pas la non-fuite ; il
   établit que la sentinelle n'a traversé aucun canal, ce qui est aussi ce qu'on
   observe quand le test ne mesure rien.

Le point 4 est ce qui matérialise R4 : il rend visible, en test, que la
protection de 1Password porte sur le canal MCP et **pas** sur le fichier monté.

### UI-4 — Branchement et démonstration (conditionnel à `verdict == OK`)

Si et seulement si UI-1 rend `OK`. Aucun code de production attendu : le substrat
MCP est générique et le serveur est un stdio ordinaire. Le livrable est la
démonstration d'AC1 sur un Environment **de test** — `list_environments`, puis
`create_local_env_file` — avec la trace, exécutée dans le process jetable d'UI-2.

Vérification d'AC3 sur pièce : une écriture (`append_variables`) doit faire
apparaître le prompt d'approbation, et son refus doit faire échouer l'appel côté
agent. L'agent n'a **structurellement** aucun chemin vers l'app desktop pour
s'auto-approuver : AC3 est tenu par le tiers, et la vérification consiste à le
constater, pas à l'implémenter.

### UI-5 — Documentation (AC4) + correction des dérives

Livrable **dans les deux branches** du verdict.

Créer `docs/mcp.md` — il n'existe pas, et la doc de référence de fait
(`docs/architecture.md:410-487`) est périmée sur trois points. Y consigner : la
portée opérateur-globale réelle et sa conséquence d'isolation (R3), le tableau de
disponibilité par mode (conversation/A2A oui ; silent/team/delegate non), la
procédure de démo par process jetable (UI-2), et le verdict daté de la sonde
(UI-1) avec sa condition de réveil.

Corriger les quatre dérives mesurées :

| Fichier | Dérive | Correction |
|---|---|---|
| `docs/architecture.md:419` | « configured in `{agent_home}/mcp.json` » | cascade opérateur (mika#1737) |
| `docs/architecture.md:414` | « rmcp (v0.17) » | rmcp 2.0 (`Cargo.toml:116`) |
| `docs/architecture.md:478-485` | « CLI ask mode : Yes » | faux depuis mika#1727 ; et pas de shutdown gracieux côté serveur |
| `templates/skills/mcp/system_prompt.md:3` | « `~/.mika/mcp.json` » | **c'est le texte que Mika sert à l'opérateur qui lui demande comment configurer MCP** — la dérive la plus coûteuse des quatre |

Synchroniser `crates/mika-agent/docs/` (`scripts/sync-agent-docs.sh`) ; le job CI
`docs-sync` échoue sinon.

---

## Verification Contract

| # | Vérification | Commande / preuve | Critère |
|---|---|---|---|
| V1 | Verdict de sonde consigné | section datée de `docs/mcp.md` | l'un de `OK` / `BLOQUÉ-AMONT` / `BLOQUÉ-TIERS` / `BLOQUÉ-SESSION`, avec sorties brutes |
| V2 | Le fichier global n'a pas été touché | `ls ~/.config/mika/mcp-servers.json` | absent, ou identique à son état antérieur |
| V3 | Non-fuite, canaux durables | test UI-3, points 1–2 | zéro occurrence de la sentinelle |
| V4 | Non-fuite, journal sous capture armée | test UI-3, point 3, `MIKA_LOG_LLM_BODIES=1` | zéro occurrence |
| V5 | **Contrôle positif** | test UI-3, point 4 | la sentinelle **est** trouvée via le canal fichier — un V3/V4 vert sans V5 ne prouve rien |
| V6 | AC1 démontré *(si `OK`)* | trace d'UI-4 | `list_environments` + montage `.env` sur Environment de test |
| V7 | AC3 démontré *(si `OK`)* | trace d'UI-4 | l'écriture déclenche le prompt ; le refus fait échouer l'appel |
| V8 | AC5 — rail pilote intact | `git diff --stat main…HEAD` | aucun fichier d'authentification pilote touché ; aucune modification sous `skills/bundled/_shared/` |
| V9 | Non-régression | `make test`, `make lint`, `make fmt` | vert |
| V10 | Doc synchronisée | `scripts/sync-agent-docs.sh` puis `git diff --exit-code` | aucun delta (job CI `docs-sync`) |

**Sur V8.** Le rail pilote reste sur abonnement : le plan ne touche aucun chemin
d'authentification de pilote, et la vérification est une absence de diff, pas un
test — c'est la forme correcte pour une non-régression dont le coût de
vérification doit rester nul.

---

## Definition of Done

- La sonde UI-1 a produit un verdict écrit et daté, avec ses sorties brutes.
- La démo, si elle a eu lieu, s'est faite dans un process jetable ; le fichier de
  configuration global est inchangé (V2).
- Le test négatif d'UI-3 est en place, avec son contrôle positif, et passe.
- `docs/mcp.md` existe et porte le verdict, la procédure d'isolation et le tableau
  de disponibilité par mode ; les quatre dérives sont corrigées ; `docs/` et
  `crates/mika-agent/docs/` sont synchronisés.
- Les trous du substrat découverts en chemin sont ouverts en tickets de suivi
  plutôt qu'absorbés : scoping per-agent des serveurs MCP ; ordre d'assemblage
  faisant échapper les outils MCP à `[tools].disabled` et au gate compact ;
  `--env` absent de `mika mcp add` ; absence de shutdown gracieux côté serveur ;
  absence de timeout sur `tools/list` ; absence de surface opérateur
  (`audit_events` / grep-signal) pour MCP.
- `make test`, `make lint`, `make fmt` verts.

---

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2281 :

1. Un agent Mika configuré avec le serveur MCP 1Password Environments peut lister ses Environments et monter un `.env` ; la démonstration se fait sur un Environment de test, jamais sur un coffre réel de Vincent.
2. Aucune valeur de secret n'apparaît dans le contexte LLM, les logs (`server.log`, corps de requête/réponse inclus) ni les transcripts : test négatif qui greppe les valeurs d'un Environment de test dans tous ces flux et exige zéro occurrence.
3. Toute opération d'écriture passe par l'approbation dans l'app 1Password (gate humain) ; l'agent ne peut pas la contourner. Cohérent avec la doctrine des actes souverains hors bus.
4. Documentation : comment installer le binaire sur gentux (Gentoo), configurer `mcp_servers` pour un agent, et ce que couvre / ne couvre pas ce chemin (les crons sans session interactive ne sont pas couverts : autre décision, service account).
5. Le rail des pilotes reste sur abonnement ; ce ticket ne change aucune authentification de pilote.

**Où chacun atterrit, et à quelles conditions :**

| AC | Unité | Statut |
|---|---|---|
| 1 | UI-4 | **Conditionnel.** Précondition détenue par un tiers (R1, R2). Le plan garantit un verdict mesuré, pas un verdict favorable. |
| 2 | UI-3 (V3–V5) | **Tenu, et élargi.** Le canal MCP est garanti par le fournisseur ; le test couvre en plus le canal réellement exposé — le `.env` monté (R4). |
| 3 | UI-4 (V7) | **Tenu gratuitement.** Le gate est détenu par le tiers ; l'agent n'a aucun chemin vers l'app desktop. Vérifié, non implémenté. |
| 4 | UI-5 | **Tenu.** L'énoncé « installer le binaire » est rectifié en « mettre à jour l'app desktop » (R1) ; « configurer pour un agent » en « process jetable » (R3). La réserve sur les crons est déjà tenue par le substrat (`mcp_manager: None` en mode silent). |
| 5 | V8 | **Tenu par non-action.** Vérifié par absence de diff. |

---

## Ce que ce travail ne fait PAS

- **Il ne rend pas le serveur 1Password fonctionnel sur gentux.** Deux
  préconditions sont hors du dépôt : une app desktop à mettre à jour, et un
  contrôle d'identité du client dont le défaut Linux est ouvert chez l'éditeur.
- **Il n'ajoute pas de scoping per-agent des serveurs MCP** (refus motivé
  ci-dessus). L'isolation passe par une procédure, et le ticket de suivi est
  ouvert.
- **Il ne déplace pas l'append MCP derrière `apply_agent_tool_visibility`.** C'est
  le correctif juste pour le contournement de denylist, et il touche l'ordre
  d'assemblage du schéma d'outils de chaque tour — ticket de suivi, pas un effet
  de bord de ce ticket.
- **Il ne scrubbe pas les sorties MCP vers le LLM** (refus motivé).
- **Il n'élargit pas l'allowlist d'environnement des enfants stdio.** Même si Q3
  montre qu'il manque `DBUS_SESSION_BUS_ADDRESS`, l'élargir change la posture de
  **tous** les serveurs MCP.
- **Il ne bascule pas sur un serveur communautaire à service account** (refus
  motivé : il inverse AC2 et AC3). Le chemin headless est la « autre décision »
  que le ticket nomme lui-même.
- **Il n'ajoute pas de surface opérateur MCP** (`audit_events`, grep-signal) : réel
  manque, mesuré, mais orthogonal — ticket de suivi.

---

## Sources

**Code — lu à HEAD `75cd9288` :**
- `crates/mika-common/src/mcp_config_path.rs:4-6, 26-27, 61-91, 96-98` — disposition ratifiée, cascade, legacy
- `crates/mika-agent/src/mcp/mod.rs:53-57, 95-121, 165-196, 199-207, 219-281, 264-270, 354-418` — fail-open, namespacing, `env_clear` + allowlist, timeouts, conversion
- `crates/mika-agent/src/mcp/config.rs:147-194, 231-258, 263-307` — loaders strict/fail-open, migration, validation
- `crates/mika-agent/src/agent_loop/mod.rs:4450-4459, 4513-4515, 5942, 7664, 7668-7675` — ordre d'assemblage, contournement de denylist et du gate compact, exclusion du mode silent
- `crates/mika-agent/src/server/mod.rs:596-612, 1797-1812` — cycle de vie, absence de shutdown gracieux
- `crates/mika-agent/src/secret_scrubber.rs:5-6, 40-80` — sortie LLM non scrubbée, 14 motifs
- `crates/mika-agent/src/tool_execution/dispatch.rs:281-353, 595-611` — persistance, `tool_source`, timeout 30 s
- `crates/mika-cli/src/cli.rs:1156-1175` ; `crates/mika-cli/src/commands/mcp.rs:146` — absence de `--env`
- `crates/mika-agent/tests/eval/harness.rs:469-472` — point d'injection sans appelant
- `Makefile:1` — `INSTALL_DIR ?= $(HOME)/.local/bin`

**Mesures locales sur gentux, 2026-09-20 :** `ls /opt/1Password/` (pas de
`onepassword-mcp`, horodatage 2023-07-20) ; `grep -i mcp
/opt/1Password/after-install.sh` (aucune occurrence) ; `which op onepassword-mcp`
(absents) ; `/var/db/pkg/app-admin/` (hors Portage) ; `~/.config/mika/`
(inexistant).

**Éditeur et communauté, consultés le 2026-09-20 :** documentation « 1Password
Environments MCP Server » (1password.dev) — outils exposés, prérequis app desktop,
transport stdio uniquement, garantie de non-retour des valeurs, montage `.env` par
FIFO en mémoire ; fil communautaire « 1Password MCP on Linux rejecting MCP
connections » — `parent process verification failed`, `Rejecting MCP connection
from pid`, diagnostic parent non-root, escalade éditeur non résolue, contournement
wrapper non garanti ; rapport de repackaging omettant le groupe setgid
`onepassword-mcp` — mécanisme `SO_PEERCRED`.

**Tickets liés :** mika#1737 (portée opérateur-globale, disposition ratifiée) ;
mika#1727 (thin client — `mika ask` ne porte plus MCP) ; mika#2282 (doublon
fermé, contenu reporté).
