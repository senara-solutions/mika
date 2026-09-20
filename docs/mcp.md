# MCP (Model Context Protocol) — portée, isolation, et le cas 1Password

Mika est **client** MCP. Elle se connecte à des serveurs externes au démarrage
(`rmcp` 2.0, transports stdio et Streamable HTTP), découvre leurs outils et les
expose au modèle sous le préfixe `mcp__{serveur}__{outil}`.

Ce document répond à trois questions que l'architecture générale ne tranche
pas : **pour qui** un serveur MCP est-il activé, **comment** en faire tourner un
sans l'imposer à toute la flotte, et **ce que vaut** le chemin « 1Password
Environments » demandé par mika#2281.

> La description du client lui-même (transports, namespacing, isolation
> d'environnement des processus enfants, limites de résultat) vit dans
> [architecture.md § 8](architecture.md#8-mcp-client-model-context-protocol).
> Ce document ne la répète pas ; il porte ce qui manquait.

---

## 1. La configuration MCP est OPÉRATEUR-GLOBALE, pas per-agent

C'est le fait le plus contre-intuitif du système, et celui sur lequel le ticket
mika#2281 s'est lui-même trompé en héritant d'une documentation périmée.

Depuis mika#1737, la disposition ratifiée est écrite en tête du résolveur
(`crates/mika-common/src/mcp_config_path.rs`) :

> *Ratified architectural disposition: MCP config is CLI/operator-side
> (edit-time), **not per-agent** or spirit-runtime.*

Le chemin per-agent `{agent_home}/mcp.json` est **legacy** : il est migré une
fois vers le chemin opérateur, puis n'est plus lu.

### Chaîne de résolution (premier atteint gagne)

| Rang | Source | Chemin |
|---|---|---|
| 1 | `MIKA_MCP_CONFIG` | la valeur, telle quelle (chemin absolu attendu) |
| 2 | `$XDG_CONFIG_HOME` | `$XDG_CONFIG_HOME/mika/mcp-servers.json` |
| 3 | `$HOME` | `$HOME/.config/mika/mcp-servers.json` |
| 4 | repli CWD | `./mcp-servers.json` (émet un `warn!`) |

Le format reste celui de `mcp.json` (convention Claude Desktop), à dessein :
un opérateur peut recopier son fichier à la main d'un rang à l'autre.

### Ce que ça implique, et qui surprend

1. **Un serveur écrit dans `~/.config/mika/mcp-servers.json` est chargé pour
   TOUS les agents** — `init_agent` le lit pour chacun, mika de production
   inclus. Il n'y a pas de « le donner à un seul agent » par fichier.
2. **`[tools].disabled` de l'`identity.toml` ne peut pas le refuser.** Les
   définitions d'outils MCP sont ajoutées par un `extend_from_slice` **après**
   `apply_agent_tool_visibility`, qui s'exécute à l'intérieur
   d'`inject_skills_and_resolve_tools`. Le denylist per-agent ne les voit donc
   jamais. Le même ordre fait que les outils MCP échappent aussi au gate
   compact-provider.
3. **Il n'existe aucun allowlist/denylist per-agent des serveurs ni des outils
   MCP.** Ce n'est pas un réglage oublié : c'est une capacité absente.

> **Conséquence opérationnelle.** Tant que le scoping per-agent n'existe pas,
> *« tester avec tel agent, jamais en prod »* signifie **process jetable**, et
> **non** *agent différent*. La procédure est en § 3.

---

## 2. Disponibilité par mode d'exécution

| Contexte | MCP ? | Site |
|---|---|---|
| `POST /message` (Telegram, webhooks GitHub → tour de conversation) | **Oui** | `server/handlers.rs` |
| `/a2a/{agent}` (`message/send`, `message/stream`) — donc **tout `mika ask`** | **Oui** | `server/a2a.rs` |
| `mika chat` (TUI, en process) | **Oui** | `mika-cli/src/commands/chat.rs` via `init::connect_mcp` |
| Mode silencieux (heartbeat, reflection, reminder, callback, deferred) | **Non** | `agent_loop/mod.rs` — `None`, *« MCP tools excluded from silent mode »* |
| Runs d'équipe | **Non** | `teams/engine.rs` passe `None` aux deux sites de spawn |
| `delegate_task` | **Non** | `tools/delegate_task.rs` passe `None` |

**Le cas `mika ask` mérite une phrase, parce que la lecture naïve est fausse
dans les deux sens.** Depuis mika#1727, `mika ask` n'exécute plus rien en
process : c'est un client léger qui poste sur `/a2a/{agent}` du démon local.
Le tour tourne donc **chez mika-spirit** et utilise le `McpManager` de
mika-spirit — pas une connexion par invocation. La réponse « oui » est donc
juste, et le mécanisme qu'on lui prêtait ne l'est pas. Ce que ça change pour un
opérateur : **après une édition de la configuration MCP, c'est mika-spirit
qu'il faut redémarrer**, pas la commande qu'il faut relancer.

**La réserve d'AC4 de mika#2281 — « les crons sans session interactive ne sont
pas couverts » — est donc déjà tenue par le substrat**, pas par une consigne :
tout tour silencieux part avec `mcp_manager: None`. Un serveur MCP n'est
joignable ni par un heartbeat, ni par un rappel, ni par un callback de
dispatch. Le chemin headless est une autre décision (compte de service), hors
périmètre.

---

## 3. Faire tourner un serveur MCP sans l'imposer à la flotte

La seule isolation disponible aujourd'hui est **procédurale** : un process
`mika` jetable dont `MIKA_MCP_CONFIG` pointe un fichier temporaire. Le rang 1
de la cascade l'emporte, donc ce process voit le serveur et **aucun autre
process ne le voit**. Le fichier global reste vierge.

```bash
# 1. Une configuration jetable, hors de tout chemin lu par le démon.
probe_dir=$(mktemp -d)
cat > "$probe_dir/mcp-servers.json" <<'JSON'
{
  "mcpServers": {
    "monserveur": {
      "transport": "stdio",
      "command": "/chemin/absolu/vers/le/binaire",
      "args": [],
      "enabled": true
    }
  }
}
JSON
chmod 600 "$probe_dir/mcp-servers.json"

# 2. Un process jetable — `mika chat` exécute son tour EN PROCESS, donc il lit
#    bien cette configuration. `mika ask` ne conviendrait pas : son tour
#    tourne chez mika-spirit, qui lit le fichier global (voir § 2).
MIKA_MCP_CONFIG="$probe_dir/mcp-servers.json" mika chat --agent mika-test

# 3. Nettoyage.
rm -rf "$probe_dir"
```

**Vérification que le fichier global n'a pas bougé** — c'est la seule preuve
qui compte :

```bash
ls -l ~/.config/mika/mcp-servers.json   # absent, ou inchangé
```

### Trois pièges, mesurés

- **`mika mcp add` écrit dans le fichier GLOBAL.** Il appelle
  `save_operator_shell()`, donc il ne sert pas à une démo isolée — sauf si
  `MIKA_MCP_CONFIG` est posé sur la même invocation. Pour un essai, éditer le
  JSON jetable à la main est plus sûr.
- **`mika mcp add` n'expose pas `--env`.** Le champ `env` du fichier existe et
  est honoré, mais la commande code en dur `env: None`. Tout serveur qui a
  besoin d'une variable impose donc d'éditer le JSON à la main.
- **Un serveur lent au handshake retarde le démarrage.** `connect_all` est
  `await`-é dans `init_agent` avec un handshake borné à 30 s. Un serveur qui
  attend une approbation graphique peut donc ajouter jusqu'à 30 s au démarrage
  de **tout** process qui le charge — raison de plus pour ne jamais poser ce
  genre de serveur dans le fichier global lu par le démon.

### Ce qui échoue en silence

`connect_all` est **fail-open** : un serveur qui ne se connecte pas est
simplement absent, sans erreur au niveau de l'agent. Le seul signal est un
`warn!` dans le journal :

```bash
grep -E 'connected to MCP server|failed to connect to MCP server' "$MIKA_SPIRIT_LOG_FILE"
```

**L'absence de ligne d'erreur ne vaut pas succès** — elle vaut aussi « aucun
serveur n'était configuré ». La ligne à chercher est la positive.

---

## 4. Frontière de secret : ce que MCP protège, et ce qu'il ne protège pas

Un serveur MCP qui détient des secrets peut garantir qu'il ne renvoie **jamais**
de valeur à son client — c'est le contrat de « 1Password Environments », et
Mika n'a rien à implémenter pour l'obtenir.

**Ce contrat porte sur le canal MCP, et sur lui seul.** En particulier, un outil
qui *monte un fichier `.env`* met ses valeurs à portée de tout agent qui sait
lire un fichier (`file-reader` est dans les allowlists usuelles). Et la copie
durable n'est protégée que sur **14 formes connues** (`SECRET_PATTERNS`) : un
mot de passe arbitraire ou une chaîne de connexion est persisté en clair dans
`tool_calls.output`. La sortie d'outil vue par le modèle, elle, n'est jamais
scrubbée — c'est une doctrine écrite (`secret_scrubber.rs`), pas un oubli.

La garde est donc **procédurale** : ne pas monter un `.env` dans un répertoire
que l'agent peut lire.

Cette frontière est mesurée par
`crates/mika-agent/tests/eval/test_mcp_secret_boundary_2281.rs`, qui fait
tourner un serveur MCP stdio factice
(`crates/mika-agent/tests/fixtures/mcp_secret_boundary_server.py`) reproduisant
le contrat de l'éditeur. Deux tests, qui n'ont de sens qu'ensemble :

1. **Négatif** — une valeur sentinelle détenue par le serveur n'apparaît ni dans
   `tool_calls.input`/`output`, ni dans `messages.metadata`, ni dans aucun corps
   de requête LLM — *et* les noms de variables, eux, traversent bien.
2. **Contrôle positif** — la **même** sentinelle, atteinte par le canal
   *fichier*, **est** trouvée. Sans lui, un test tout-vert n'établirait pas la
   non-fuite : il établirait que la sentinelle n'a traversé aucun canal, ce
   qu'on observe aussi quand l'instrument ne mesure rien.

Le fixture reproduit le *contrat*, pas le produit : ce test porte sur le chemin
MCP **générique** et vaut pour tout serveur présent et futur.

---

## 5. Sonde 1Password Environments — verdict du 2026-09-20 : `BLOQUÉ-AMONT`

mika#2281 demandait qu'un agent Mika puisse lister ses Environments 1Password
et monter un `.env`. La sonde s'arrête à sa première question.

### Q1 — le binaire existe-t-il sur gentux ?

```console
$ ls -l /opt/1Password/onepassword-mcp
ls: cannot access '/opt/1Password/onepassword-mcp': No such file or directory

$ ls -ld /opt/1Password
drwxr-xr-x 1 nobody nobody 1056 Aug 11  2023 /opt/1Password

$ ls /opt/1Password/ | grep -i mcp
(aucune — 29 entrées au total)

$ which op onepassword-mcp
op not found
onepassword-mcp not found

$ grep -c -i mcp /opt/1Password/after-install.sh
0

$ ls -d /var/db/pkg/app-admin/*1[Pp]assword*
(aucune correspondance — installation hors Portage)
```

**Verdict : `BLOQUÉ-AMONT`.** Le binaire s'appelle `onepassword-mcp` (et non
`1password-mcp` comme l'écrit le ticket) ; il n'est **pas** distribué séparément
mais **bundlé dans l'application desktop** ; et l'application installée ici date
d'août 2023, hors du gestionnaire de paquets. Que `after-install.sh` ne
contienne aucune occurrence de `mcp` le confirme sur pièce : ce script est
précisément l'endroit où les versions récentes posent le groupe
`onepassword-mcp` et son `setgid`.

**Il n'y a donc rien à « installer » au sens de l'AC4 du ticket : il y a une
application desktop de trois ans de retard, posée à la main hors Portage, à
mettre à jour.** C'est un geste d'opérateur sur la machine, pas une ligne de
code, et c'est la première précondition de tout le reste.

Q2 (« le serveur accepte-t-il un client Mika ? ») et Q3 (« l'approbation
est-elle atteignable depuis le process ? ») sont **inatteignables** tant que Q1
échoue. Elles ne sont pas réputées échouer : elles ne sont pas posées.

### Le second obstacle, connu et non levé

Même une fois l'application à jour, un obstacle documenté attend, et il est
**chez l'éditeur** :

- Le serveur vérifie l'identité de son client par `SO_PEERCRED` (groupe
  `onepassword-mcp` + `setgid` posés par `after-install.sh`) **et** par une
  vérification du processus parent.
- Les échecs observés en production sur Linux portent
  `parent process verification failed: Binary Permissions` et
  `Rejecting MCP connection from pid …`.
- Le diagnostic communautaire convergent est qu'**un parent qui n'appartient
  pas à root est rejeté**.

Or `Makefile` pose `INSTALL_DIR ?= $(HOME)/.local/bin` : **mika-spirit est
déployé exactement là où le rejet est documenté, et pour la même raison.** Ce
n'est pas un risque spéculatif, c'est un échec attendu par construction, dont
la cause est chez un tiers et qu'aucune ligne de ce dépôt ne corrige. Au
2026-09-20 le défaut est escaladé chez l'éditeur et non résolu.

**Le contournement par script wrapper n'est pas à tenter.** Il est donné comme
non garanti par l'éditeur, et son effet est de maquiller le parent pour
franchir un contrôle d'authentification — c'est-à-dire de contourner
exactement la vérification d'identité du client qui donne sa valeur à l'AC3 du
ticket (l'approbation humaine dans l'application).

### Condition de réveil

Rejouer la sonde quand **les deux** conditions sont réunies :

1. l'application desktop 1Password de gentux est à jour et
   `ls -l /opt/1Password/onepassword-mcp` rend un binaire ;
2. le défaut Linux de vérification du parent est annoncé corrigé par
   l'éditeur, **ou** un essai montre que le rejet ne se produit pas.

La sonde reprend alors à Q2, telle qu'écrite dans le plan
`docs/plans/2026-09-20-002-feat-2281-agent-mcp-mika-consomme-le-serveur-mcp-plan.md`.

### Ce qui n'a PAS été fait, et pourquoi

- **Aucune bascule vers un serveur MCP communautaire adossé à `op` + compte de
  service.** Ces serveurs fonctionnent en headless, ce qui les rend séduisants
  pour les crons — et ils **inversent la propriété même que le ticket
  achète** : les valeurs transitent en clair par le canal MCP et vers le
  modèle. Y basculer au motif que l'officiel résiste, ce serait satisfaire
  l'AC1 en détruisant l'AC2 et l'AC3.
- **Aucun scoping per-agent des serveurs MCP** (§ 1). Ce serait revenir sur la
  disposition ratifiée de mika#1737 et déplacer l'ordre d'assemblage du schéma
  d'outils de chaque tour — un ticket de substrat à part entière, pour une
  démonstration qui n'a pas encore prouvé qu'elle pouvait se connecter.
- **Aucun scrubbing des sorties MCP vers le modèle.** Ce serait inverser une
  doctrine écrite pour *tous* les outils, sur 14 motifs qui ne couvrent de
  toute façon pas un secret arbitraire. Le remède au vrai risque est
  procédural (§ 4), pas un filtre.
- **Aucun élargissement de l'allowlist d'environnement des enfants stdio.**
  Elle transmet `XDG_RUNTIME_DIR` mais ni `DBUS_SESSION_BUS_ADDRESS`, ni
  `WAYLAND_DISPLAY`, ni `DISPLAY` — ce qui pourrait manquer à un serveur devant
  afficher une approbation graphique. L'élargir changerait la posture de
  sécurité de **tous** les serveurs MCP, et se décide séparément.

---

## 6. Limites connues du client MCP

Réelles, mesurées, et non corrigées ici — chacune a son ticket de suivi :

| Limite | Effet |
|---|---|
| Pas de scoping per-agent des serveurs | Un serveur est visible de tous les agents (§ 1) |
| Outils MCP ajoutés après `apply_agent_tool_visibility` | `[tools].disabled` ne peut pas les refuser ; le gate compact-provider ne les voit pas non plus |
| `mika mcp add` n'expose pas `--env` | Toute variable nécessaire impose d'éditer le JSON à la main |
| Pas d'arrêt gracieux côté serveur | `McpManager::shutdown()` n'est appelé que par `mika chat` ; mika-spirit s'en remet à `kill_on_drop` |
| Pas de timeout sur `tools/list` | Seul le handshake est borné (30 s) ; un serveur qui accepte puis se tait sur la découverte bloque l'initialisation de l'agent |
| Pas de surface opérateur | Aucun `audit_events`, aucun signal de grep dédié : le seul instrument est le `warn!` de connexion |

Un appel d'outil MCP, lui, est borné : 30 s (`TOOL_TIMEOUT_SECS`), comme tout
outil builtin.
