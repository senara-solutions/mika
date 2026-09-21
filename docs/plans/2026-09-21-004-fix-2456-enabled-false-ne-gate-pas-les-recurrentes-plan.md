# mika#2456 — un agent désactivé ne produit aucun tour automatique récurrent

**Ticket :** senara-solutions/mika#2456
**Type :** fix (p2, santé-substrat)
**Branche :** `fix/2456/p2-substrat-enabled-false-ne-gate-pas`

---

## 1. Ce que la lecture du code rectifie du ticket

Deux mesures déplacent le diagnostic, et chacune change le remède. Elles sont le
premier livrable de ce grooming.

### R1 — Il n'existe aucune porte `enabled` niveau agent. Il n'y a rien à étendre.

Le ticket écrit que « la porte `enabled` ne couvre que le chemin interactif »
(corps) et, en commentaire 2, que ce knob est « lu par le chemin **interactif**
uniquement ». **Les deux sont faux : ce champ n'existe nulle part.**

`prompt.rs::Identity` (`crates/mika-agent/src/prompt.rs:694-716`) énumère ses dix
champs : `name`, `emoji`, `reflection`, `heartbeat`, `kg`, `skills`, `tools`,
`context`, `session`, `curator`. **Aucun `enabled` racine.** Une recherche
exhaustive de `agent_enabled` / `is_enabled` / `disable_agent` sur les trois
crates ne rend que `disable_agent_provisioning` (un réglage de provisionnement,
pas d'exécution) et des `enabled` de skills, de KG, de MCP.

### R2 — `identity.toml` de mika-test ne porte pas ce que le ticket y a lu.

`MIKA_TEST_IDENTITY` (`well_known_agents.rs:1464-1485`) porte `[kg] enabled =
false` et `[skills] nudge_enabled = false`. Le ticket les a lus comme un
`enabled = false` racine « + `nudge_enabled = false` ». Ce sont deux sous-clés de
sections sans rapport avec l'activité de l'agent.

**Conséquence qui décide la forme du correctif.** `Identity` ne porte pas
`deny_unknown_fields` : un opérateur qui écrit `enabled = false` à la racine
d'un `identity.toml` aujourd'hui obtient un parse **réussi** et un champ
**ignoré**. C'est la classe mika#2205 / mika#2293 — *un réglage qu'on ne peut pas
observer n'est pas un réglage, c'est un espoir* — sauf qu'ici il n'est même pas
lu. Le correctif doit donc **poser la clé à l'endroit exact où le ticket croyait
la lire**, sinon il crée un second knob et laisse le premier inerte.

### R3 — Ce que le ticket établit correctement, et qui reste entier

Le symptôme est exact et le commentaire 2 vise juste sur le mécanisme :

| récurrente | site | gaté ? |
|---|---|---|
| `heartbeat` | `server/mod.rs:1616` + `mika-cli/src/commands/chat.rs:229` | par `[heartbeat] enabled` (défaut `true`) |
| `reflection` | `server/mod.rs:1627` + `chat.rs:244` | par `[reflection] enabled` (défaut `false`) |
| `curator_review` | `server/mod.rs:1751-1767` | **non gaté — aucun knob n'existe** |
| `auto_pull_groomed`, `wip_rescue`, `qa_review_reconcile`, `worktree_reap` | `server/mod.rs:1640-1749` | par variable d'environnement, mika-dev seulement |

Le coût mesuré est réel : un tour heartbeat de mika-test de **59 s** finissant en
`reasoning budget exhausted`, zéro texte visible, ~5,18 $/22 h sur le tenant
cloud.

---

## 2. La décision centrale : deux moitiés, et la seconde n'est pas une redondance

### 2.1 Pourquoi la garde à l'enregistrement ne suffit pas

Le réflexe est d'envelopper les neuf sites d'enregistrement dans un `if
agent_enabled`. **Deux mesures le refusent.**

**(a) `ensure_recurring_task` ressuscite ce que la garde vient d'annuler.**
`task_engine/mod.rs:54-65` appelle `revert_config_cancel_recurring_task(label)`
**avant** de créer, précisément pour lever le veto mika#1742 sur une row annulée
par un knob (mika#2271). Donc un seul appelant non gardé — le site CLI
`chat.rs:229`, ou n'importe quel site futur — **rouvre la row** que le boot
venait d'annuler. Une garde répartie sur N sites dont l'un lève le veto des
autres n'est pas une garde, c'est une course.

**(b) Neuf sites, et rien ne fait rougir le dixième.** Un site ajouté sans la
garde ne rend aucune décision fausse : il rend la garde inerte, avec tous les
tests au vert. C'est la forme exacte de ce que mika#2205 a dû fermer
(`auto_pull` / `wip_rescue` lisant le PAT à côté du résolveur canonique).

### 2.2 La garde vit DANS `ensure_recurring_task`, pas chez ses appelants

`ensure_recurring_task` est déjà le **passage obligé** des neuf enregistrements,
et déjà l'endroit qui lève le veto. Y porter la garde ferme la classe entière
par construction, site CLI et sites futurs compris :

```rust
pub async fn ensure_recurring_task(
    db: &AsyncDatabase,
    home_dir: &Path,        // ← ajouté
    label: &str,
    cron_expr: &str,
    action_config: &str,
)
```

Le paramètre est **ajouté à la signature, jamais dérivé** : les neuf appelants
tiennent déjà le `home_dir` (`agent_state.home_dir`, `ctx.home_dir`), et un
appelant qui l'oublie **ne compile pas**. Le compilateur, pas un relecteur, est
ce qui force un futur site à prendre la décision — motif
`dispatch_substrate_diagnostic`, et motif `mika2334_every_scan_variant_is_covered`.

Quand l'agent est désactivé, la fonction **n'appelle pas** le revert, **ne crée
pas** la row, et **annule** la row existante par
`cancel_recurring_task_by_label`. C'est ce qui rend vrai le test négatif du
ticket (« aucune tâche récurrente `recurring_active` »).

### 2.3 Le filet au tir, et pourquoi il est nécessaire APRÈS la garde

`dispatcher.rs::dispatch_run_skill` (`:572-606`) est le point unique où les sept
triggers builtin sont routés. Un refus y est posé pour les tâches dont
`trigger_type == "recurring"`.

**Ce n'est pas une redondance** (argument mika#2279, garde A / garde B) : la row
peut être `recurring_active` sans être jamais passée par la garde 2.2 — row née
avant le déploiement, row ressuscitée par un chemin non encore inventorié, ou
`enabled` basculé à `false` pendant que le moteur tourne (la garde 2.2 ne
s'exécute qu'au boot et au provisionnement). Le filet borne la fuite de coût
**sans attendre un redémarrage**, ce que la garde seule ne fait pas.

**Régime attendu du filet : zéro ligne.** Toute occurrence nomme une row admise
sans passer par la garde, donc un chemin d'enregistrement à établir — c'est un
signal d'attribution, pas un compteur de trafic.

---

## 3. Le knob

### 3.1 Site : `identity.toml`, racine, opérateur-owned

```toml
# ~/.mika/agents/<name>/identity.toml
enabled = false
```

**Racine et non `[agent] enabled`** : c'est la forme exacte que le ticket et
l'opérateur ont déjà écrite (R2). Poser la clé ailleurs laisserait la forme
intuitive silencieusement ignorée, ce qui est le défaut d'origine avec une étape
de plus.

**`identity.toml` et non `customer_config`** — mika#2358 a choisi
`customer_config` pour `proactive_daily_budget` sur un argument précis (« rien ne
le réécrit au démarrage »), qui ne transporte pas ici. Trois raisons :

1. **Le frère de ce knob y vit déjà.** `[heartbeat] enabled` et `[reflection]
   enabled` sont dans `identity.toml`. Mettre la porte générale ailleurs que ses
   deux portes particulières crée deux sites pour une même nature de décision.
2. **Le levier doit être posable hors ligne, agent arrêté.** `customer_config`
   est une table de la base de l'agent ; `identity.toml` s'édite sur un pod à
   l'arrêt et se lit dans l'inventaire.
3. **`customer_config` est réglable par le modèle** (`SETTABLE_CONFIG_KEYS` *est*
   la surface d'outil). Un agent désactivé pouvant se réactiver lui-même n'est pas
   une porte. mika#2358 nomme ce risque et l'accepte pour un budget de fréquence ;
   il n'est pas acceptable pour un interrupteur d'activité.

**Non code-owned** : la clé n'entre **pas** dans `CODE_OWNED_IDENTITY_SECTIONS`.
Elle rejoint `name`, `emoji`, `[reflection]`, `[kg]` — préservés verbatim. C'est
une décision d'opérateur, et la rendre code-owned la rendrait ré-écrasable au
démarrage suivant, c'est-à-dire le pire mode de panne pour un interrupteur
(argument mika#2329 sur le rejet du toggle `identity.toml` pour le STOP à chaud —
ici le risque est levé par l'absence de réconciliation, pas contourné).

### 3.2 Trois états, et l'absence ne ferme pas

| valeur | résolution |
|---|---|
| absente | `true` — back-compat ; tout agent déployé reste actif |
| `true` / `false` | honorée |
| illisible (type non-booléen) | parse `identity.toml` échoue → chemin *malformed* existant, inchangé |

Règle mika#2023 appliquée telle quelle : *unrecognized values fail closed,
absence does not.* Une absence est la forme légitime de chaque agent existant ;
en faire un `false` couperait la flotte au déploiement.

### 3.3 L'identité fail-closed garde `enabled = true`, et le refus est cité

`load_identity` rend `fail_closed_identity()` sur fichier absent ou illisible
(mika#2027). La tentation est d'y poser `enabled = false` : un agent à zéro skill
qui brûle 59 s de LLM est du gaspillage.

**Refusé**, et le précédent est littéral. Le doc-comment de `load_identity` a
déjà tranché la question jumelle, mot pour mot :

> *Whether the fail-closed path wants a stricter denylist than mika-arch's
> steady-state one is a real question, and a separate one... Left open
> deliberately rather than answered in a load-path fix.*

Élargir la sévérité du fail-closed dans un correctif de coût ferait qu'une erreur
I/O transitoire sur un `identity.toml` couperait les messages proactifs d'un
tenant famille — un blast radius large, silencieux, et sans rapport avec la fuite
mesurée. **Question nommée, renvoyée à son ticket.**

### 3.4 Composition avec les knobs par-feature : conjonction

`enabled` niveau agent **ET** knob par-feature. Le levier
`[heartbeat] enabled = false` reste en vigueur (le pod cloud mika-test le porte
aujourd'hui, preuve du commentaire 1) et n'est ni retiré ni déprécié.

### 3.5 Portée : les récurrentes, pas les sollicitées

Couvert : les tâches `trigger_type = "recurring"`, donc les sept triggers builtin
— heartbeat, reflection, curator_review **et** les quatre scans mika-dev. La
garde est sur l'agent, pas sur une liste de features : un agent désactivé ne tire
rien.

**Conséquence nommée** : poser `enabled = false` sur mika-dev arrête toute la
boucle autonome. C'est le comportement voulu d'un agent désactivé, et il est
écrit plutôt que découvert.

Non couvert, délibérément : les **reminders** (`SilentTrigger::Reminder`) et le
chemin interactif. Un reminder est *sollicité* — l'utilisateur l'a demandé — et
le borner reviendrait à lui refuser ce qu'il a demandé ; argument repris mot pour
mot de mika#2358 (« Borner `Reminder` reviendrait à refuser à l'utilisateur ce
qu'il a explicitement demandé, c'est-à-dire l'inverse du défaut à corriger »).
Le chemin interactif reste ouvert : mika-test est un **banc d'essai**, et un
`mika ask --agent mika-test` doit continuer de répondre sous `enabled = false`
— sans quoi le knob détruit l'usage même de l'agent qui a servi de preuve.

---

## 4. Requirements

- **R-1** — `Identity` porte un champ racine `enabled: bool`, `#[serde(default =
  "default_agent_enabled")]` rendant `true`.
- **R-2** — `ensure_recurring_task` prend `home_dir` et refuse l'enregistrement
  quand l'agent est désactivé ; le refus **annule** la row existante et
  **n'appelle pas** `revert_config_cancel_recurring_task`.
- **R-3** — Les neuf appelants passent `home_dir`. Un appelant qui l'omet ne
  compile pas.
- **R-4** — `dispatch_run_skill` refuse les tâches `trigger_type ==
  "recurring"` d'un agent désactivé, avant tout appel LLM et avant toute prise
  de `agent_lock`.
- **R-5** — Le chemin interactif, les reminders, les callbacks et les tours
  `run_skill` non récurrents sont **inchangés**.
- **R-6** — Aucun agent ne reçoit `enabled = false` dans ce travail. Le ticket
  le dit : *« Décision runtime = opérateur. »*
- **R-7** — `enabled` n'entre pas dans `CODE_OWNED_IDENTITY_SECTIONS`.
- **R-8** — Observabilité, § 5.
- **R-9** — Documentation : `docs/configuration.md` § identity.toml + l'entrée
  correspondante du CLAUDE.md racine.

---

## 5. Surfaces opérateur

Trois événements dans `$MIKA_SPIRIT_LOG_FILE`, plus une ligne d'audit.

- **`agent_recurring_gate_resolved`** (INFO, dédupliqué sur l'état résolu par
  agent) — champs `agent_id`, `enabled`, `source` ∈ `{identity, default}`.
  **Répond à « cet agent est-il désactivé, et par quelle porte ? » sans lire le
  disque.** Modèle et raison : `llm_budget_resolved` (mika#2293),
  `tenant_language_resolved` (mika#2247). `source: "default"` sur un agent qu'on
  croit désactivé ⇒ la clé n'a pas atterri, la cause est dans le fichier, **pas**
  dans la garde. Dédupliqué : une répétition à l'identique est tue, un
  **changement** est ré-émis.

- **`recurring_registration_refused_agent_disabled`** (INFO — `agent_id`,
  `label`, `cancelled_rows`) — un enregistrement refusé par la garde 2.2.
  **Régime attendu : non vide au premier démarrage suivant la pose du knob** (une
  ligne par récurrente annulée), puis une poignée par démarrage. Sans cette ligne,
  une garde qui mord se lit exactement comme une garde inerte (mika#2205).

- **`recurring_fire_refused_agent_disabled`** (WARN — `agent_id`, `task_id`,
  `label`) — le filet 2.3 a tiré. **Régime attendu : zéro ligne** hors de la
  première fenêtre post-déploiement (où les rows nées avant la garde sont encore
  actives). Toute occurrence durable nomme un chemin d'enregistrement qui échappe
  à `ensure_recurring_task` : **établir ce chemin, ne pas élargir le filet.**

- **SQL** — `SELECT target_key, count(*) FROM audit_events WHERE tool_name =
  'agent_recurring_gate' GROUP BY 1;`. Une ligne par refus **au tir** uniquement
  (le refus à l'enregistrement est borné par la cadence des démarrages ; le tir
  ne l'est pas). Dédupliquée sur 24 h par `(agent, label)` — doctrine mika#2131 :
  l'information durable est « cet agent est gaté », pas « il l'était encore à
  14 h 32 ».

---

## 6. Implémentation

1. **`crates/mika-agent/src/prompt.rs`** — champ `enabled` sur `Identity`,
   `default_agent_enabled() -> bool { true }`, valeur `true` dans
   `Identity::default()` **et** dans `fail_closed_identity()` (§ 3.3), doc-comment
   portant R2 et le refus § 3.3.
2. **`crates/mika-agent/src/task_engine/mod.rs`** —
   `agent_recurring_tasks_enabled(home_dir) -> (bool, GateSource)` à côté de
   `heartbeat_enabled_for_agent` ; garde + branche cancel + les deux événements en
   tête de `ensure_recurring_task` ; signature étendue.
3. **`crates/mika-agent/src/server/mod.rs`** — sept sites : passage de
   `&agent_state.home_dir`.
4. **`crates/mika-cli/src/commands/chat.rs`** — deux sites : `&ctx.home_dir`.
5. **`crates/mika-agent/src/task_engine/dispatcher.rs`** — filet en tête de
   `dispatch_run_skill`, conditionné à `task.trigger_type == "recurring"`, avant
   le `match trigger_name` et avant toute acquisition de `agent_lock`.
6. **Documentation** — `docs/configuration.md` § identity.toml (tableau des
   champs + les trois états + la conjonction § 3.4 + la portée § 3.5) ;
   `CLAUDE.md` racine, entrée voisine de `MIKA_AGENT_TIER`.

---

## 7. Verification Contract

### Tests comportementaux

- **V1** — `enabled = false` : `ensure_recurring_task` ne crée rien et la row
  préexistante devient `cancelled`. *Le test négatif littéral du ticket.*
- **V2** — **Contrôle négatif, porteur** : `enabled = true` et clé **absente** →
  la row est créée normalement. Sans lui, V1 ne distingue pas « la garde mord »
  de « la fonction est cassée ».
- **V3** — Anti-résurrection : agent désactivé, row `cancelled`, second appel à
  `ensure_recurring_task` → la row **reste** `cancelled`
  (`revert_config_cancel_recurring_task` n'a pas été appelé). *C'est le défaut
  mika#2271 retourné, et le cœur de § 2.1(a).*
- **V4** — Filet : row `recurring_active` d'un agent désactivé, tâche due →
  `dispatch_run_skill` refuse, aucun appel LLM, `agent_lock` non pris.
- **V5** — Non-régression : un `run_skill` **non** récurrent d'un agent désactivé
  s'exécute (R-5).
- **V6** — Conjonction : `enabled = true` + `[heartbeat] enabled = false` → le
  heartbeat reste refusé (R-3.4).
- **V7** — Fail-closed : `identity.toml` absent → `enabled` résolu à `true`,
  les récurrentes sont enregistrées (§ 3.3).

### Test structurel

- **V8** — `CODE_OWNED_IDENTITY_SECTIONS` ne contient pas `enabled` (R-7). Un
  ajout futur ferait ré-écrire le knob de l'opérateur au démarrage suivant.

L'exhaustivité des appelants **n'a pas de test** : elle est portée par le
compilateur (§ 2.2), ce qui est plus fort qu'un scan de source.

### Sonde post-déploiement, et ses trois haltes

Sur mika-test, poser `enabled = false`, redémarrer, puis à 24 h :

1. `agent_recurring_gate_resolved` rend `{enabled: false, source: "identity"}`.
2. `mika tasks list --agent mika-test` : aucune `recurring_active`.
3. Aucune session `heartbeat-*` ni `curator_review` nouvelle.
4. `mika ask --agent mika-test "ping"` **répond** (R-5 — le banc reste un banc).

**Halte 1 — `source: "default"`.** La clé n'a pas atterri (faute de frappe,
mauvais fichier, agent servi depuis un autre home). **Ne pas toucher à la
garde** : la cause est dans le fichier.

**Halte 2 — aucune ligne `agent_recurring_gate_resolved` alors que l'agent a
tourné.** Le binaire servi est antérieur au correctif — classe mika#2340.
**Établir le déploiement avant toute conclusion sur le code** (`cat
~/.mika/skills/.manifest-writer`).

**Halte 3 — `recurring_fire_refused_agent_disabled` persiste au-delà de la
première fenêtre.** Un chemin d'enregistrement échappe à `ensure_recurring_task`.
**Ne pas élargir le filet** : l'établir, et l'y ramener.

**Contrôle négatif de la flotte** : sur mika-dev, mika-qa et mika-arch (aucune
clé posée), `agent_recurring_gate_resolved` rend `{enabled: true, source:
"default"}` et aucune récurrente ne bouge.

---

## 8. Ce que ce travail n'achète pas

- **Aucune économie tant que l'opérateur ne pose pas la clé.** Le ticket le pose
  lui-même (« Décision runtime = opérateur »). Ce travail livre le levier, pas
  la décision.
- **Aucune détection de clé inconnue.** Un `enabld = false` (faute de frappe)
  reste silencieusement ignoré : `Identity` n'a pas `deny_unknown_fields`, et
  l'ajouter ferait échouer le parse de toute identité portant une clé future,
  donc fail-closer toute la flotte sur une clé en avance de version. La ligne
  `agent_recurring_gate_resolved` est ce qui rend la faute de frappe visible —
  par `source: "default"` — plutôt qu'un refus de parse. **Ticket de suivi.**
- **Le silence ne prouve rien si personne ne relit.** Aucun compteur n'est
  ajouté au-delà du § 5 ; la seule mesure est la sonde ci-dessus.

---

## 9. Hors périmètre, délibérément

- **`[curator] enabled`** — le commentaire 2 refuse explicitement « d'exiger une
  sous-clé par feature », et la porte générale rend le cas mesuré inutile à
  couvrir deux fois. Reste vrai et nommé : le curator est aujourd'hui la seule
  récurrente sans **aucun** knob par-feature, donc on ne peut pas le couper sans
  couper l'agent. **Ticket de suivi**, conditionné à une mesure — un agent actif
  dont on veut couper le seul curator.
- **Poser `enabled = false` sur mika-test** (R-6).
- **Les reminders et le chemin interactif** (§ 3.5).
- **Élargir le fail-closed de `load_identity`** (§ 3.3).
- **Le coût du tour heartbeat lui-même** — les 59 s finissant en `reasoning
  budget exhausted` avec zéro texte sont un défaut de budget de sortie sur
  glm-5.3, pas un défaut de porte. Ce travail empêche le tour d'avoir lieu ; il
  n'explique pas pourquoi il coûte tant quand il a lieu.

---

## Definition of Done

- [ ] `Identity.enabled` livré avec son défaut `true` et son doc-comment.
- [ ] `ensure_recurring_task` garde + annule + ne ressuscite pas ; signature
      étendue ; neuf appelants migrés.
- [ ] Filet `dispatch_run_skill` sur `trigger_type == "recurring"`.
- [ ] Les trois événements et la ligne d'audit du § 5.
- [ ] V1 à V8 verts ; `cargo test`, `cargo clippy`, `cargo fmt` propres.
- [ ] `docs/configuration.md` et `CLAUDE.md` racine à jour ; `docs-sync` vert.
- [ ] Aucun `identity.toml` d'agent modifié ; `CODE_OWNED_IDENTITY_SECTIONS`
      inchangée.

---

## Acceptance criteria

Dérivées du corps du ticket (§ Attendu et § Test négatif — le ticket n'a pas de
section `## Acceptance criteria`).

- **AC1** — Un agent dont `identity.toml` porte `enabled = false` ne porte
  **aucune** tâche récurrente en statut `recurring_active` après le démarrage
  suivant : ni `heartbeat`, ni `reflection`, ni `curator_review`.
- **AC2** — Ce même agent ne tire **aucun** tour automatique récurrent : aucune
  session `heartbeat-*`, aucun `curator_review`, aucun appel LLM issu d'une
  récurrente. Le refus est posé avant tout appel LLM.
- **AC3** — Un agent dont `identity.toml` porte `enabled = true` **ou** ne porte
  pas la clé conserve exactement ses récurrentes d'aujourd'hui — cadence, labels
  et statuts inchangés.
- **AC4** — Un enregistrement refusé ne peut pas être ressuscité par un autre
  appelant de `ensure_recurring_task` tant que l'agent reste désactivé.
- **AC5** — Le chemin interactif d'un agent désactivé reste servi : `mika ask
  --agent <name>` répond.
- **AC6** — L'état de la porte est lisible dans le journal sans lecture du
  disque, avec sa provenance (`identity` / `default`).
- **AC7** — Aucun agent existant ne change de comportement au déploiement :
  aucune identité n'est modifiée et l'absence de la clé vaut `true`.
