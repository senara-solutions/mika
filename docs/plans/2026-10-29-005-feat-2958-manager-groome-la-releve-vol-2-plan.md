# mika#2958 — le manager groome la relève : vol 2a, le rail A2A comme objet testable

> **Ticket** : `mika issue#2958` — Phase 2 (c) du manager, vol 2.
> **Vol 1** : livré par la PR #2960, commit `8d127552e`, mergé. Il a posé le geste
> (`groom_trigger.rs`, 598 lignes) : l'interrupteur, le vocabulaire des huit motifs,
> les quatre sondes, le trait `GroomSender`, l'exception nommée au scan. Il ne porte
> **pas** le rail.
> **Ce vol (2a)** : le rail A2A, comme objet testable, dans le fichier qui porte déjà
> l'exception. Sans constante d'environnement, sans `.env.example`, sans câblage de
> cycle, sans rendu de rapport — ceux-là sont les vols 2b et 2c, nommés par la ligne
> `Tracked in:` en fin de plan.
> **Plan antérieur du ticket** : `docs/plans/2026-10-29-003-feat-2958-manager-groome-la-releve-plan.md`
> (il portait le vol 1, et a lui-même redécoupé le ticket).

---

## 1. Ce qui est déjà acquis, et qu'il ne faut pas refaire

Trois lectures du dépôt, faites avant ce plan, retirent du périmètre trois choses
que la lecture naïve du ticket y mettrait.

### 1.1 AC1 est livrée du côté du scan, et l'allowlist est fermée à trois

`no_dispatch_test.rs` porte **déjà** le jeton `"message/send"` dans
`FORBIDDEN_TOKENS` (l. 59-65) **et** son exception nommée dans
`EXEMPTED_OCCURRENCES` (l. 104-108) :

```rust
(
    "groom_trigger.rs",
    "message/send",
    "mika#2958 — déclenche le groom de la relève par la voie moteur (A2A vers mika-dev, jamais un spawn), derrière MIKA_MANAGER_GROOM_WRITE, OFF par défaut",
),
```

Et `mika2958_lallowlist_porte_les_trois_gestes_et_pas_un_de_plus` épingle
`len() == 3` : « Une écriture de plus est un autre ticket, avec son propre seuil
d'ouverture ».

Deux conséquences, toutes deux contraignantes :

1. **Le rail doit vivre dans `groom_trigger.rs`.** Un fichier voisin qui porterait
   le POST aurait besoin d'une quatrième entrée, que le pin refuse. Ce n'est pas une
   gêne : c'est la contrainte qui garde le geste à un seul site.
2. **`no_dispatch_test.rs` n'a rien à changer dans ce vol.** La raison inscrite au
   vol 1 décrit déjà le rail (« A2A vers mika-dev, jamais un spawn »), donc elle ne
   devient pas stale quand le rail arrive — elle devient vraie.

Par la même lecture, les deux tests structurels de `mod.rs` —
`mika2700_le_vol_1_nouvre_aucun_chemin_decriture` (qui scrute `relief.rs`,
`reporter.rs`, `mika#2700`) et
`mika2958_le_vol_2_declare_son_interrupteur_avec_sa_table_de_verite` (qui épingle
`declarants == ["groom_trigger.rs"]` et trois besoins d'API) — **restent verts sans
modification**. Le premier a déjà rougi et été amendé dans le commit du vol 1 ; le
second passe déjà. Le vol 2a n'en touche aucun.

> **Ce que la PR du vol 2a dira donc sur AC1** : le scan n'a pas bougé, parce qu'il
> avait déjà bougé — au vol 1, dans le même commit que le docstring, exactement comme
> l'AC le demandait. Ce vol est celui où l'exception cesse d'être une promesse.

### 1.2 Le jeton d'auth existe, et il n'en faut pas un second

Les routes `/a2a/{agent_name}` sont protégées par
`auth::require_internal_token` (`server/mod.rs:386-397`), qui compare le bearer à
`state.internal_token` — c'est-à-dire `MIKA_INTERNAL_TOKEN`. Le rail présente **ce**
jeton. Un second nom pour le même secret ne crée pas un canal : il crée une occasion
de divergence silencieuse entre ce que le serveur vérifie et ce que le manager
envoie.

### 1.3 L'URL est déclarée, jamais dérivée — et pas dans ce vol

`budget_floor.rs::local_a2a_agent` (`mika-cli`, l. 20-82) sait reconnaître une URL
`/a2a/{name}` en loopback. Il vit dans **un autre crate**, inatteignable depuis
`mika-agent`, et le réimplémenter serait dériver une URL au lieu de la déclarer —
exactement ce que mika#2249 et mika#2368 refusent. Les trois rails sortants voisins
du module (`ENV_DELIVERY_URL`, `ENV_ESCALATION_URL`, `ENV_LIVENESS_URL`) sont tous
déclarés et aucun n'est borné par un prédicat de forme.

La déclaration de `ENV_GROOM_URL` est **au vol 2b**, avec son site d'appel. C'est la
frontière que le vol 1 a déjà argumentée et que ce vol reprend telle quelle :

> « Déclarer `ENV_GROOM_URL` au vol 1 serait déclarer une URL que rien ne lit :
> exactement l'« interrupteur dont la branche ON ne fait rien » que ce module
> refuse. Le rail arrive donc **avec son site d'appel**, au vol 2. »

Le même argument, appliqué une fois de plus, place la **constante** au vol où elle
est lue par `manager_config_from_env` et passée au cycle. Le vol 2a livre le rail
comme un objet qui **reçoit** son adresse, son jeton et son budget — ce qui est aussi
la ligne de coupe que le vol 1 a déjà ratifiée pour les candidats (`deja_groome` et
`groom_en_cours` sont des **paramètres** de `trigger_relief_grooms`, pas des lectures).

---

## 2. Périmètre du vol 2a

Deux fichiers de code, plus une entrée de manifeste sous `docs/`.

### 2.1 `A2aGroomSender` — le rail

Dans `crates/mika-agent/src/milestone_manager/groom_trigger.rs`, après le trait
`GroomSender` et avant `trigger_relief_grooms`.

```rust
pub struct A2aGroomSender {
    client: reqwest::Client,
    url: String,
    token: String,
}

impl A2aGroomSender {
    /// `url`, `token` et `budget` sont **reçus**, jamais lus ici.
    pub fn new(url: String, token: String, budget: Duration) -> Self
}
```

Patron copié trait pour trait sur `HttpLivenessSink` (`liveness.rs:184-268`), le rail
sortant déjà déclaré de ce même module :

- `reqwest::Client::builder().timeout(budget).build()` avec
  `.unwrap_or_else(|_| reqwest::Client::new())` — un client qui ne se construit pas
  ne doit pas empêcher un cycle de **lecture** du manager de tourner.
- `self.client.post(url)` et **jamais** `Method::POST` ni le littéral `"POST"` :
  `no_dispatch_test.rs` interdit `"\"POST\""` dans tout fichier du module (l. 44), et
  les deux autres formes le porteraient.
- `.bearer_auth(&self.token)`.
- Sur erreur de transport, `e.without_url()` et **jamais** `{e}` : le `Display` de
  `reqwest::Error` appende « for url (…) », et le manifeste d'egress affirmera que
  l'URL ne part pas au journal.

**Pourquoi `budget` est un argument de `new`, et non une constante du fichier.** Ce
point est le seul qui restait ouvert avant rédaction ; il est tranché ici, et la
raison est un lint.

`scripts/check-a2a-timeout-literals.sh` porte deux règles sur ce périmètre :

- **A1** — l'argument de `.timeout(` ne peut pas être un *littéral* de durée, et
  l'expression est **évaluée** (`from_secs(10 * 60)` est refusé) ; `from_secs(CONST)`
  est admis. Une variable l'est donc aussi.
- **A2** — un `const … : Duration` ≥ 60 s dans le périmètre doit être nommé
  `DEFAULT_*` **et** vivre dans un fichier qui déclare aussi sa variable d'env
  (`*_ENV: &str = "MIKA_…"` ou `env::var("MIKA_…")`), ou figurer dans
  `scripts/a2a-timeout-allowlist.txt`.

Or ce budget **doit majorer** le budget de tour de mika-dev (~300 s), donc il est
≥ 60 s par construction. Les trois issues, et leur arbitrage :

| issue | coût | retenue |
|---|---|---|
| (i) `DEFAULT_GROOM_SEND_TIMEOUT` + `ENV_GROOM_SEND_TIMEOUT_SECS` dans le fichier | recrée au vol 2a une constante d'env que ce vol a décidé de ne pas déclarer, + un bloc `.env.example`, + ~20 lignes | non |
| (ii) `budget: Duration` en argument de `new` | aucun `const` dans le périmètre ⇒ A2 muet ; A1 satisfait (variable, pas littéral) | **oui** |
| (iii) entrée dans `scripts/a2a-timeout-allowlist.txt` | cette allowlist est livrée vide et mesurée vide ; y entrer est l'anti-patron que le lint condamne | non |

(ii) est aussi la plus cohérente avec la ligne de coupe : le rail reçoit **les trois**
faits de sa configuration (url, jeton, budget) et n'en lit aucun. Le vol 2b les
résout ensemble, au même endroit, à partir de l'environnement.

**Pourquoi un timeout trop court est la pire issue, et pourquoi le plan le dit ici.**
Le geste est **synchrone** (§ 2.3). Si le manager abandonne la requête avant que le
tour de mika-dev ait rendu la main, le tour continue côté mika-dev, le groom part,
et le manager le compte `send_failed` — puis le relance au cycle suivant. Ni l'une ni
l'autre moitié n'est fausse localement ; ensemble elles produisent un double groom
sous un rapport qui dit « échec ». Le budget passé par le vol 2b devra donc majorer
le budget de tour, et c'est le commentaire du paramètre qui le dira.

### 2.2 `build_send_body` — la fonction pure qui garde le piège

```rust
pub(crate) fn build_send_body(instruction: &str, message_id: &str) -> serde_json::Value
```

Rend l'enveloppe JSON-RPC complète : `jsonrpc`, `id`, `method: GESTE`, et
`params.message` (`role`, `parts: [{kind: "text", text: instruction}]`, `messageId`,
`kind`).

Trois propriétés, et chacune a son test :

1. **`method` vaut `GESTE`**, la constante déjà déclarée à plat au vol 1 — pas une
   chaîne retypée. Le docstring du vol 1 dit pourquoi la forme éclatée
   (`.method("message").arg("send")`) est interdite : elle échappe au grep plat du
   garde. Le rail est le premier *lecteur* de cette constante, ce qui est précisément
   ce qu'elle attendait.
2. **`configuration` est ABSENT du corps**, et c'est le test qui garde le piège.
   `server/a2a.rs:1225-1240` : la branche `returnImmediately` « creates the task row
   and returns it in `submitted`; it **never runs the agent loop** ». Un manager qui
   la prendrait créerait une tâche qui ne groome rien, pendant que son rapport dirait
   « groom déclenché ». L'absence du champ n'est pas un oubli : c'est le choix, et un
   test l'assère nommément (`assert!(body["params"].get("configuration").is_none())`).
3. **`message_id` est un argument**, pas un `Uuid::new_v4()` interne. C'est ce qui
   rend la fonction déterministe, donc assertable sur son corps entier.

Patron : la raison écrite de `classify_delivery_auth` (`cadence.rs:338`) — « passing
it in rather than reading the environment here keeps the function pure and testable ».

### 2.3 `classify_send_outcome` — la fonction pure de classification

```rust
pub(crate) fn classify_send_outcome(
    status: u16,
    body: Option<&serde_json::Value>,
) -> Result<(), GroomSendError>
```

Table de lecture, dans l'ordre :

| fait observé | classe | pourquoi |
|---|---|---|
| `status` 2xx, pas de champ `error` | `Ok(())` | — |
| `error.code == AGENT_BUSY` (−32000) | `GroomSendError::AgentBusy` | code numérique = champ structuré (mika#2179) |
| `error.code` autre, dont `INTERNAL_ERROR` (−32603) | `GroomSendError::Other` | voir ci-dessous |
| `status` 401 ou 403 | `GroomSendError::CredentialRefused` | patron `liveness.rs:250-253` |
| autre non-2xx | `GroomSendError::Other` | — |

La constante vient de `mika_a2a::jsonrpc::AGENT_BUSY` — **importée, jamais retypée**.
C'est le même symbole que `a2a_wait_queue::busy_error` émet, donc il n'y a qu'un
seul endroit où le nombre existe.

**La classification est sur le CODE, jamais sur le message.** `busy_error` et
`legacy_busy_error` portent tous deux le message `"Agent is busy"` ; seul le code les
sépare (−32000 contre −32603). Dériver la classe d'un sous-texte est exactement ce
que mika#2179 refuse.

**Conséquence nommée, parce qu'elle est une lecture honnête et non un défaut.** Quand
la file A2A est désarmée (`a2a_queue_enabled = false`), le refus busy arrive en
`legacy_busy_error()` — code −32603, indistinguable d'une panne interne. Le rail le
classe donc `Other`, et le motif au rapport sera `send_failed` et non `agent_busy`.
Le plan le dit plutôt que de le deviner par le message : un instrument qui devine est
un instrument qui mentira le jour où le message change. Le docstring de la fonction
porte cette phrase.

**Le geste reste synchrone.** Il tient parce que le tour `groom` de mika-dev
*dispatche* un claude-pilot d'arrière-plan et rend la main : le tour est court, ce
n'est pas le groom entier qui est attendu. `build_send_body` n'émettant aucune
`configuration`, la branche `returnImmediately` n'est jamais prise et le verrou
d'agent (`try_take_slot` / `wait_for_agent_lock`) s'applique — c'est lui qui produit
le refus busy que la table ci-dessus lit.

### 2.4 `mod.rs` — le ré-export et le déplacement de la frontière

- `pub use groom_trigger::A2aGroomSender;` à côté des ré-exports existants du module.
- Le paragraphe du docstring de frontière, amendé : le vol 2a a livré le rail et ses
  deux fonctions pures ; ce qui reste dû est **le câblage** (constante d'env,
  `.env.example`, `spawn.rs`, `cadence.rs`, lecture d'idempotence, champ de
  checkpoint) et **le rendu ON** (`reporter.rs`). Le docstring de `groom_trigger.rs`
  (l. 30-60) reçoit la même correction à sa dernière phrase, qui dit aujourd'hui que
  « le rail A2A, le câblage de cycle, la lecture « un groom est en cours » et le rendu
  de rapport […] sont le vol 2 » : le rail en sort.

**Aucun nouveau test structurel dans `mod.rs`.** La propriété de site unique du
littéral `message/send` est déjà gardée par le scan du vol 1, et la table de vérité de
l'interrupteur par `mika2958_le_vol_2_declare_son_interrupteur_avec_sa_table_de_verite`.
Ajouter un troisième scan reviendrait à garder deux fois la même chose, et à livrer un
détecteur là où le ticket n'en demande pas.

### 2.5 `docs/egress/egress-manifest.toml` — l'entrée `[[sink]]`

Obligatoire, et le lint le rendra tel : `scripts/lib/egress_manifest_lint.py` détecte
les sites client par `CLIENT_CONSTRUCTION_RE` (`Client::new()`, `Client::builder()`,
`ClientBuilder::new(`), donc le `Client::builder()` du § 2.1 exige son entrée. Le
fichier est sous `docs/`, donc hors du compte de lignes.

```toml
[[sink]]
id = "manager-groom-a2a"
destination_source = "config"
class = "internal"
logged = true
client_site = "crates/mika-agent/src/milestone_manager/groom_trigger.rs"
call_site = "crates/mika-agent/src/milestone_manager/groom_trigger.rs"
owner = "@samidarko"
```

Deux champs valent leur justification :

- **`destination_source = "config"`** — l'URL est entièrement dynamique (reçue en
  argument, résolue au vol 2b depuis l'environnement). C'est aussi ce qui exempte
  l'entrée du contrôle anti-fantôme D4, qui ne saurait pas confronter une destination
  qu'aucun littéral ne porte.
- **`logged = true`** — l'assertion prudente. Le numéro d'issue entre bien au journal
  par la sonde `manager_groom_triggered` (champ `issue`). Ni l'URL ni le jeton n'y
  entrent, par construction : `e.without_url()` au site d'erreur, et le jeton ne passe
  que par `.bearer_auth`. `logged = true` décrit donc la donnée *métier* qui part et
  est tracée ; déclarer `false` serait affirmer une absence que la sonde contredit.

---

## 3. Ce que le vol 2a ne livre pas, et pourquoi

| non livré | pourquoi pas ici |
|---|---|
| `ENV_GROOM_URL` + bloc `.env.example` | une constante d'env sans lecteur est l'interrupteur dont la branche ON ne fait rien (§ 1.3) |
| `spawn.rs` (ré-export, `manager_config_from_env`, sonde) | c'est le lecteur de la constante ci-dessus |
| `cadence.rs` (`with_groom_sender`, site d'appel) | le câblage pèse à lui seul ~200 lignes avec ses tests ; rail + câblage donnerait ~575 estimées, au-dessus de la borne |
| la lecture « un groom est en cours » (AC2) | elle est une lecture de `GhRunner` + checkpoint, orthogonale au rail |
| `GroomTriggerOutcome::est_muet()` + rendu ON (AC4) | une propriété sans lecteur est le même anti-patron ; elle part avec `reporter.rs`, au vol 2c |
| toute modification de `no_dispatch_test.rs` | l'allowlist est fermée à trois et sa raison décrit déjà ce rail (§ 1.1) |

Les AC2, AC3 et AC4 sont donc **portées par les vols 2b et 2c**, nommés par la ligne
`Tracked in:`. Les décisions de conception déjà prises pour eux, et qui ne doivent pas
être reprises à zéro, sont consignées au § 8 pour que le vol suivant les hérite plutôt
que de les redécouvrir.

---

## 4. Ordre de commit, et le plancher du vol

Trois commits, dans cet ordre. Le **plancher** est le commit 1 : il est mergeable
seul, et laisse l'arbre dans un état où rien n'a changé de comportement.

1. **`build_send_body` + `classify_send_outcome` + leurs tests.** Deux fonctions
   pures, `pub(crate)`, aucun réseau, aucun client. Le commit le plus dense en
   propriétés et le moins dense en risque. Si le vol doit s'arrêter là, il a livré ce
   qui garde le piège `returnImmediately` et la classification sur le code, et le vol
   2b n'a plus qu'à les brancher.
2. **`A2aGroomSender` (struct, `new`, `impl GroomSender`) + son test de
   construction.** C'est le commit qui introduit `Client::builder()`, donc celui qui
   doit porter l'entrée du manifeste d'egress — les deux dans le même commit, sinon le
   lint rougit sur le commit intermédiaire.
3. **`mod.rs` (ré-export + frontière) et la dernière phrase du docstring de
   `groom_trigger.rs`.**

---

## 5. Contrat de vérification

Tout est hors réseau. Les tests de `liveness.rs` — le rail le plus proche — n'ouvrent
**aucun serveur HTTP** : tout passe par des fakes et des fonctions pures. Le vol 2a
tient la même discipline, et c'est ce qui le rend estimable.

Tests inline en `#[cfg(test)] mod tests` de `groom_trigger.rs` (Porte 1, mika#1947 :
tout fichier sous `src/milestone_manager/**` est DECISION-CORE et le module n'a aucun
répertoire `tests/` imbriqué).

**Corps de requête** — contre `build_send_body` :

1. `method` vaut exactement `GESTE`.
2. le texte de la seule `part` vaut l'instruction reçue, telle quelle.
3. **`params.configuration` est absent** — le test du piège, avec le numéro de ticket
   et la raison en commentaire.
4. `messageId` vaut l'argument reçu (déterminisme).

**Classification** — contre `classify_send_outcome` :

5. 200 sans `error` ⇒ `Ok(())`.
6. `error.code == AGENT_BUSY` ⇒ `AgentBusy`, et `.classe() == "agent_busy"`.
7. `error.code == INTERNAL_ERROR` ⇒ `Other`, et `.classe() == "send_failed"` — le test
   qui **fige la lecture honnête** du § 2.3 : il assère que le refus de la file
   désarmée n'est pas maquillé en `agent_busy`.
8. 401 ⇒ `CredentialRefused`. 9. 403 ⇒ `CredentialRefused`. 10. 500 sans corps ⇒ `Other`.

**Construction du rail** :

11. `A2aGroomSender::new` avec un budget quelconque rend un rail dont l'URL et le
    jeton sont ceux reçus — le contrôle que rien n'est lu de l'environnement.
12. Contrôle négatif de non-régression : `trigger_relief_grooms(false, Some(&rail), …)`
    avec un `A2aGroomSender` pointant sur une adresse **non routable** rend un outcome
    `default()` et n'émet aucune requête. Le rail est présent, armé à `false` : c'est
    la forme qui distingue « désarmé » de « poseur mort ». (Le test ne dépend pas de
    l'adresse : le refus de cycle précède toute construction de requête — c'est
    précisément ce qu'il assère.)

**Lints et gardes à repasser verts, sans modification** :

- `cargo test -p mika-agent milestone_manager` — dont `no_dispatch_scaffolding_in_milestone_manager`,
  `mika2698_une_exception_ne_couvre_quun_seul_jeton_du_fichier`,
  `mika2958_lallowlist_porte_les_trois_gestes_et_pas_un_de_plus`,
  `mika2700_le_vol_1_nouvre_aucun_chemin_decriture`,
  `mika2958_le_vol_2_declare_son_interrupteur_avec_sa_table_de_verite`, et les quatre
  tests de la section « Relève à grommer » du reporter (`reporter.rs:2267`, `:2362`,
  `:2404`, `:2436`).
- `bash scripts/check-a2a-timeout-literals.sh` — A1 et A2 (§ 2.1).
- `bash scripts/verify-egress-manifest.sh` — lockstep bidirectionnel (§ 2.5).
- `cargo clippy --all-targets`, `cargo fmt --check`.
- `bash scripts/verify-pipeline.sh origin/main`.

---

## 6. Risques, et ce qui les borne

| risque | borne |
|---|---|
| le lint egress rougit sur le commit 2 | l'entrée `[[sink]]` est dans **ce** commit, pas dans un suivant (§ 4) |
| A2 rougit sur une constante de timeout | aucune constante : le budget est un argument (§ 2.1) |
| le littéral `"POST"` entre par `Method::POST` | `client.post(url)`, comme `liveness.rs:220-224`, et le scan du module le refuse de toute façon |
| l'URL fuit au journal par `{e}` | `e.without_url()`, et le manifeste l'affirme |
| un quatrième jeton d'écriture apparaît | le pin `len() == 3` rougit ; c'est le signal, pas l'obstacle |
| le rail devient un second lecteur de l'interrupteur | il n'en lit aucun : `armed` reste un paramètre de `trigger_relief_grooms` (doctrine du lecteur unique, mika#2201) |

---

## 7. Definition of Done

- `A2aGroomSender` existe dans `groom_trigger.rs`, implémente `GroomSender`, et reçoit
  url, jeton et budget en arguments de `new` — sans lire l'environnement.
- `build_send_body` et `classify_send_outcome` existent comme fonctions pures
  `pub(crate)`, et les douze tests du § 5 passent.
- Le corps `message/send` n'émet **aucun** champ `configuration`, et un test nommé
  l'assère avec sa raison.
- `AGENT_BUSY` est importé de `mika_a2a::jsonrpc`, et la classification ne lit aucun
  message d'erreur.
- `pub use groom_trigger::A2aGroomSender;` dans `mod.rs`, et les deux docstrings de
  frontière déplacés d'un cran.
- L'entrée `[[sink]] manager-groom-a2a` est dans `docs/egress/egress-manifest.toml`,
  dans le même commit que le `Client::builder()`.
- `no_dispatch_test.rs` est **inchangé**, et les cinq gardes structurels du § 5
  passent sans modification.
- Tous les lints du § 5 sont verts.
- La PR porte `Tracked in: senara-solutions/mika#2958` et dit, en une ligne, que le
  scan n'a pas bougé parce qu'il avait déjà bougé au vol 1.

---

## 8. Acceptance criteria

Transcrits verbatim du corps de `mika issue#2958`. Le vol 2a en porte **AC1** ; AC2,
AC3 et AC4 sont les vols 2b et 2c (§ 3 et `Tracked in:`). Les décisions de conception
déjà arrêtées pour eux sont notées sous chaque critère, pour qu'elles soient héritées
et non redécouvertes.

- **AC1.** Avec l'interrupteur OFF, aucun groom n'est déclenché. C'est une garantie
  structurelle, avec un contrôle négatif dans le même test. Le geste entre comme
  **exception nommée** au scan de `no_dispatch_test.rs`, sur le patron des entrées de
  mika#2698 vol 2 et de mika#2699 vol 4. Le scan n'est pas assoupli. Le test
  `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` est amendé dans le même commit, et
  la PR dit ce qui le faisait rougir.

  → *Porté par ce vol.* L'exception nommée et l'amendement de
  `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` sont **acquis au vol 1** (§ 1.1) ;
  ce vol ajoute la garantie structurelle côté rail (test 12 du § 5 : rail présent,
  `armed == false`, zéro requête) avec son contrôle négatif de forme — le rail
  construit et passé, ce qui distingue « désarmé » de « poseur absent ».

- **AC2.** Sélection et idempotence : un ticket déjà groomé, ou dont un groom est en
  cours, n'est pas relancé. Le plan dit par quelle lecture le manager voit un groom en
  cours : les tâches moteur `Groom …` et `ready-label`, ou autre chose. Le motif est la
  collision d'un groom manuel avec un groom moteur.

  → *Vol 2b.* La lecture est arrêtée et tient en trois couches, **pas** par les tâches
  moteur que le ticket propose : celles-ci vivent dans le SQLite auquel le manager n'a
  pas accès (il n'a ni `AsyncDatabase`, ni `AppState`, ni `resolve_agent` ; ses seuls
  rails sont `GhRunner` et des POST vers des URL déclarées). Les trois couches
  retenues : (a) le callout `Plan: docs/plans/` dans le corps du ticket ⇒
  `deja_groome` ; (b) le label `dispatch:<siège>` ⇒ `groom_en_cours` ; (c) un champ de
  checkpoint borné `grooms_declenches`, `#[serde(default)]`, sur le patron de
  `posted_flight_task_ids` + `POSTED_FLIGHTS_RETAINED` + `merge_posted()`.

- **AC3.** La voie est celle du moteur : la même que `mika ask --agent mika-dev
  "groom mika#N"`, ou son équivalent interne. Elle n'ouvre ni spawn ni dispatch
  d'implémentation. Le plan dit comment le geste reste borné : combien de grooms par
  cycle, et ce qu'il fait quand mika-dev a déjà un pilote en vol.

  → *La voie est tranchée dans ce vol* (§ 2.1 à 2.3) : `POST /a2a/mika-dev`, méthode
  `message/send`, bearer `MIKA_INTERNAL_TOKEN`, sur le spirit local — la même porte que
  `mika ask` depuis que celui-ci est un client mince (mika#1727, site unique
  `remote_ask.rs::send_message_to_agent`). Les deux bornes sont déjà déclarées :
  `GROOMS_PER_CYCLE = 1` (vol 1), qui est aussi ce que le schéma v34 autorise — un seul
  créneau `dispatch_class = 'groom'` par agent ; et, quand mika-dev a un pilote en vol,
  le refus busy existant côté A2A (`a2a_wait_queue`, mika#2163) remonte en `AgentBusy`,
  ce qui **arrête le cycle** sans retenter. Le câblage qui place ce rail au cycle est le
  vol 2b.

- **AC4.** Ce qui reste de l'ombre se lit dans le rapport. Interrupteur OFF : la
  section « Relève à grommer » est inchangée. Interrupteur ON, en test seulement : le
  rapport dit quels grooms ont été déclenchés, et lesquels ont été sautés, avec leur
  motif.

  → *Vol 2c.* La moitié OFF est tenue **dans ce vol** par le fait que rien ne touche
  `reporter.rs` et que les quatre tests de sa section restent verts (§ 5). La moitié ON
  est arrêtée : le rendu sera gouverné par une propriété du **résultat**
  (`GroomTriggerOutcome::est_muet()`, déclarée dans `groom_trigger.rs`, lecteur unique),
  jamais par une relecture de l'interrupteur dans `reporter.rs` — ce qui violerait
  mika#2201 et ferait rougir
  `mika2958_le_vol_2_declare_son_interrupteur_avec_sa_table_de_verite`. Le résultat
  atteint le rapport par un champ `groom: Option<GroomTriggerOutcome>`
  `#[serde(default)]` posé sur `assessment` après `assess`, sur le patron de
  `flight_verdicts` (`cadence.rs:885-950`) — pas par un paramètre neuf du reporter.

- **Taille** : à estimer tests compris, et à découper en vols au-delà de la borne.

  → § 9.

**Hors périmètre, dit explicitement.** Ce vol ne livre **aucun détecteur** : les douze
tests du § 5 sont comportementaux (un corps de requête, une table de classification,
une construction), aucun n'a « aucune violation trouvée » pour chemin de succès, et
aucun scan structurel n'est ajouté ni modifié. La gate mika#2306 est donc **N/A** et
la section `## Fire-Disposition` n'est pas inventée.

---

## 9. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/groom_trigger.rs` — `A2aGroomSender` (struct, `new`, docstring) | 65 |
| idem — `impl GroomSender for A2aGroomSender` (`send`) | 50 |
| idem — `build_send_body` (fonction pure + docstring du piège) | 45 |
| idem — `classify_send_outcome` (fonction pure + docstring de la lecture honnête) | 55 |
| idem — tests inline `#[cfg(test)]` (les douze du § 5) | 135 |
| `crates/mika-agent/src/milestone_manager/mod.rs` — ré-export + frontière | 28 |
| `docs/egress/egress-manifest.toml` — entrée `[[sink]]` | 0 (sous `docs/`) |

Total estimé : 378 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi la coupe est là, et pas 50 lignes plus loin.** Trois mesures la placent.

1. **La dérive mesurée de ce module est de 2,3.** `git show --stat 8d127552e` : le vol
   1 réel hors `docs/` pèse 903 lignes (37 `.env.example` + 598 `groom_trigger.rs` +
   156 `mod.rs` + 104 `no_dispatch_test.rs` + 8 `no_repo_list_test.rs`) contre 395
   estimées. La cause est structurelle et non accidentelle : ce module écrit sa
   doctrine dans ses docstrings, et une décision y coûte un paragraphe. L'estimation
   ci-dessus est donc faite **ligne à ligne contre un artefact existant du même
   module** — `HttpLivenessSink` et ses deux voisines pures font 115 lignes pour la
   même forme — et non au forfait.
2. **Rail + câblage donnerait ~575 estimées**, bien au-dessus de 429. La coupe
   rail/câblage est donc forcée, pas choisie.
3. **Une coupe à 423 laisserait 6 lignes de marge**, soit exactement la configuration
   dans laquelle trois vols sont morts au plafond le 2026-10-14. 378 laisse 51 lignes,
   et le § 4 donne un plancher mergeable (les deux fonctions pures) si la marge est
   malgré tout consommée.

Tracked in: senara-solutions/mika#2958

Le ticket reste **ouvert** et porte la suite — précédent déjà établi sur lui-même : le
vol 1 l'a redécoupé et fermé aucun ticket de suivi. Les deux vols restants :

- **Vol 2b — le câblage et l'idempotence (AC2, AC3).** `ENV_GROOM_URL` + bloc
  `.env.example` ; `spawn.rs` (ré-export, `manager_config_from_env`, sonde de
  configuration) ; `cadence.rs` (`with_groom_sender` comme sixième builder de
  `CycleContext`, le site d'appel après `write_pool_admissions`, les trois refus de
  cycle dans l'ordre) ; la lecture d'idempotence à trois couches du § 8 AC2 ; le champ
  de checkpoint borné `grooms_declenches` ; le champ `groom: Option<GroomTriggerOutcome>`
  sur `Assessment` dans `types.rs`. **≈ 370 lignes estimées.**
- **Vol 2c — le rendu de la moitié ON (AC4).** `GroomTriggerOutcome::est_muet()` dans
  `groom_trigger.rs` ; l'amendement de la section « Relève à grommer »
  (`reporter.rs:395-476`), dont le titre porte aujourd'hui « mode ombre » et ne peut
  donc pas rester tel quel sous ON ; les déclenchés et les sautés avec leur motif.
  **≈ 100 lignes estimées.**

Chacun est mergeable seul, et le vol 2a ne dépend d'aucun des deux.
