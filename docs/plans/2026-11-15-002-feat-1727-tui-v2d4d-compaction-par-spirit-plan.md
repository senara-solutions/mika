# mika#1727 — vol V2d-4d : `/compact` passe par `compacter`

- **Ticket :** senara-solutions/mika#1727
- **Branche :** `feat/1727/tui-v2d4d-compaction-par-spirit`
- **Base :** `origin/main` @ `3393bad0` (merge de V2d-4c, #3090)
- **Label :** `loop-substrate` (plafond de tours 200)
- **PR :** `Refs mika#1727`. Jamais `Closes`, `Fixes` ni `Resolves`, ni dans le corps ni dans un commit.

---

## 0. Pourquoi ce vol tient en un seul vol

Ce vol est **exactement** la moitié que V2d-4c a reportée, et son estimation est
donc déjà écrite deux fois : le plan de V2d-4c l'annonçait à **254 lignes**
estimées hors `docs/`, et le § *Taille estimée* ci-dessous la remesure poste par
poste à **230**. Les deux bornes sont tenues avec marge :

| borne | unité | ce vol | marge |
|---|---|---|---|
| 429 (par vol, mika#2693) | lignes estimées hors `docs/` et hors `CLAUDE.md` | **230** | 199 (46 %) |
| 500 (corps du ticket) | `git diff --numstat origin/main...HEAD`, plan exclu | **260** | 240 |
| 1000 (par PR, mika#2636) | idem | 260 | ne mord pas |

**Aucune seconde coupe n'est donc proposée**, et c'est le seul arbitrage que ce
plan avait à ne pas faire : le corps du ticket dit que ce vol « *est déjà le
reliquat d'une coupe, et une seconde coupe demande le mot de Vincent* ». La
clause de halte du ticket (« *si l'un des deux dépasse sa borne, le groom
s'arrête et le dit* ») est donc **non déclenchée**, sur les deux nombres.

Et le périmètre **ne peut pas** être coupé plus fin sans livrer un état que la CI
refuse — c'est un fait de compilation, pas un jugement :

- les deux `expect(dead_code)` de `session_remote.rs` portent sur `compacter` et
  `CompactedMirror`. Les retirer **avant** le câblage laisse un `dead_code` nu ;
  les garder **après** rend l'attente insatisfaite. Dans les deux cas
  `cargo clippy --all-targets --all-features -- -D warnings`
  (`.github/workflows/ci.yml:76`) rougit. Ils tombent dans le commit qui câble, ou
  le commit ne compile pas ;
- `App.llm` a **un seul** lecteur, `handlers.rs:273`, dans le corps de
  `handle_compact`. Il devient mort à l'instant exact du câblage, et `App` est
  binaire-privé (`lib.rs` n'expose pas `tui`), donc `dead_code` s'applique à ses
  champs. Le ticket interdit `allow` comme `expect` dessus : le retrait est
  **forcé par la CI**, dans le même vol que le câblage ;
- le motif `maybe_compact(` de la garde ne peut pas précéder le câblage : il
  rougirait sur du code que rien ne remplace, et la seule issue serait une entrée
  d'allowlist — ce que la Fire-Disposition interdit nommément.

Le vol est donc un **bloc indivisible**. Ce qui reste hors de lui est nommé au
§ 8 et noté `Tracked in: senara-solutions/mika#1727`.

**Ce que ce plan ne fait pas :** il n'ouvre aucun ticket (gel jusqu'au 01/12), ne
redécoupe ni V3 ni V4, et ne touche à aucun fichier du plan de contrôle.

---

## 1. Ce qui est mesuré sur `main` (@ `3393bad0`)

Tout ce qui suit a été lu, pas supposé.

| fait | site | ce qu'il décide |
|---|---|---|
| `handle_compact(app: &mut App<'_>) -> String` : refus si `app.status != AgentStatus::Idle`, puis `count_messages()`, puis `if count <= 50 { "Nothing to compact ({count}/50 messages)." }`, puis `maybe_compact(&app.db, app.llm.as_ref())` | `handlers.rs:262-276` | la fonction à réécrire en entier |
| Le seuil `50` est écrit **deux fois** en dur, l. 270 et l. 271 (la comparaison et le dénominateur du message) | `handlers.rs:270-271` | critère 3 : les deux disparaissent |
| Le serveur, lui, **lit** le seuil : `if messages <= compaction::COMPACTION_THRESHOLD { return ok(false, messages); }` | `server/compact.rs:93-95` | le client n'a plus rien à comparer : D3 |
| Le handler serveur prend le verrou de l'agent **avant** de compter, et rend 409 sur `try_lock_owned` échoué | `server/compact.rs:73-82` | le seul producteur de `Busy` |
| `count_messages()` reste légitimement appelé par `/status` | `handlers.rs:444` | il ne peut **pas** devenir un motif de garde : D6 |
| `maybe_compact(` a **un seul** site de production dans ce fichier | `handlers.rs:273` (et l. 3020, un doc-comment de test) | le rouge-avant du motif neuf nommera cette ligne, et elle seule |
| Le modèle : `handle_clear` charge le `.env` et `spirit_url()`, puis délègue à `clear_par_spirit(app, &base_url, &|nom| std::env::var(nom).ok())` | `handlers.rs:112-121` | la forme de l'enveloppe, reprise telle quelle |
| `clear_par_spirit(app: &mut App<'_>, base_url: &str, lire_env: SourceEnv<'_>) -> String` | `handlers.rs:137` | la forme de la fonction testable |
| `compacter(base_url, token, agent) -> Result<CompactedMirror, SessionRemoteIssue>`, avec `#[cfg_attr(not(test), expect(dead_code, …))]` | `session_remote.rs:352-371` | la porte à câbler, et l'`expect` qui tombe |
| `CompactedMirror { compacted: bool, messages: usize }`, même `expect` | `session_remote.rs:70-88` | le second `expect` qui tombe |
| Le doc-comment de `CompactedMirror.compacted` dit déjà « demandée **et n'a pas échoué** », jamais « une ligne `summary` a été écrite » | `session_remote.rs:72-80` | D4 n'invente rien : il **applique** un contrat déjà écrit à la porte |
| `CompactedMirror.messages` est le compte **d'avant**, dans les deux issues | `session_remote.rs:82-87` | le mot « before » du message de D4 |
| `SessionRemoteIssue::Busy` ; son `Display` dit « another turn holds its lock » | `session_remote.rs` + test l. 573 | D5 : la cause est dite par le `Display`, le remède ne l'est pas |
| `jeton_interne(lire: SourceEnv<'_>) -> Result<String, String>` — lecture **unique** du jeton dans tout le crate | `session_remote.rs:177` | aucune seconde lecture n'est écrite ici |
| Le § *État du vol* du doc de module annonce **deux** `expect`, « mesuré sous `cargo clippy -p mika-cli --all-targets` », et dit que le chiffre tombera « quand V2d-4c câblera la dernière » | `session_remote.rs:28-40` | D7 : à réécrire, et la phrase nomme même le mauvais vol |
| La garde `mika1727_v2d4b_la_tui_ne_clot_plus_la_base_en_local` : `ALLOWED` vide + `assert!(…is_empty())`, deux motifs, témoin `async fn handle_clear(`, contre-témoin `fn test_clear_singleton_does_not_end_session`, un seul `assert!` final par motif | `handlers.rs:3026-3073` | la garde à étendre (D6) |
| Son doc-comment : « `maybe_compact(` n'est pas un motif de ce vol : il appartient à V2d-4c » | `handlers.rs:3020-3024` | **faux dans les deux sens** après ce vol : à réécrire |
| `App.llm` : déclaré l. 578, paramètre l. 691, posé l. 730 (`App::new`) et l. 825 (`App::new_team`, via `dummy_provider()`) | `tui/app.rs` | les quatre sites de production du retrait |
| `use mika_common::llm::LlmProvider;` est l'**unique** usage de ce nom dans `app.rs` | `tui/app.rs:13` | l'import part avec le champ |
| `dummy_provider()` n'a que **deux** appelants dans tout le crate, tous deux dans `app.rs` (l. 825 et l. 2361) | mesuré par `grep -rn dummy_provider crates/mika-cli/src` | les deux partent ; la fonction reste dans `mika-common`, hors périmètre |
| Les trois constructeurs de test d'`App::new` : `app.rs:2354` (passe `dummy_provider()`), `handlers.rs:1533` et `input.rs:1181` (chacun son `NoopLlmProvider`) | — | le critère 6 nomme les deux `NoopLlmProvider` ; le troisième site ne coûte qu'une ligne |
| Les deux `NoopLlmProvider` : `handlers.rs:1474-1504` + import l. 1471, `input.rs:1127-1154` + import l. 1121. `ProviderKind` du même `use` reste utilisé dans les deux ; `Arc` aussi (`SkillRegistry::empty()`) | — | les deux imports se **réduisent** à `ProviderKind`, ils ne disparaissent pas |
| `#[allow(clippy::too_many_arguments)]` sur `App::new` : 14 paramètres → 13 | `tui/app.rs` | l'`allow` reste nécessaire ; ce vol n'en **ajoute** aucun et n'en retire aucun |
| Le banc `fake_spirit(&[(chemin, statut, corps)])` → `fallback` **501**, et `RequeteRecue{methode, chemin, autorisation, corps}` **sans `Debug`** | `rewind_remote.rs:540-592` | emprunté, jamais recopié ; les appels se comptent par `vues.len()` |
| Le `mod tests` de `handlers.rs` porte déjà `fake_spirit`, `RequeteRecue`, `Mutex`, `Arc`, `SourceEnv`, `avec_jeton()`, `test_app()`, et le modèle d'aide `par_le_banc` / `par_la_couture` | `handlers.rs:1464`, 1635, 2608-2614, 2646 | **zéro ligne d'import à ajouter** |
| CI : `cargo clippy --all-targets --all-features -- -D warnings` | `.github/workflows/ci.yml:76` | un warning rougit la CI ; aucun `allow` ni `expect` n'est ajouté par ce vol |

**Et les faits qui sont des absences** — elles portent autant de décisions :

| absence mesurée | comment | conséquence |
|---|---|---|
| `/compact` n'a **aucun** test | `grep -n "compact" handlers.rs` : dispatch l. 41, fonction l. 262, et rien sous `mod tests` (l. 1464+) | les sept tests sont **neufs** ; rien à rebrancher. C'est le poste qui pèse 70 % du vol |
| `DeadEndpoint` n'est **pas** importé dans `handlers.rs` | `grep -rn DeadEndpoint crates/mika-cli/src` → `budget_floor.rs`, `session_remote.rs`, `chat_turn.rs`, `chat.rs` | aucun critère n'exige un point injoignable pour `/compact` : D5 prend un **500** du banc, qui est déjà le montage du voisin (l. 2979-3000) et ne coûte pas d'import |
| Le verrou d'agent de mika-spirit **ne voit pas** le tour de la TUI | V1 n'est pas livré : `chat.rs:341` et `chat.rs:468` appellent `agent::run_agent` **dans le processus de la TUI** | c'est toute la raison de garder le refus « pas au repos » côté client : D2 |
| `handle_compact` ne **modifie** aucun champ d'`App` | lecture de `handlers.rs:262-276` : seuls `app.status`, `app.db`, `app.llm` sont lus | la fonction neuve prend `&App`, pas `&mut` : D1 |
| `grep -rn "app\.llm\|pub llm:" crates/mika-cli/src/` rend **aussi** `init.rs:17` | mesuré : `init.rs:17`, `app.rs:578`, `handlers.rs:273`, `chat.rs:935`, `chat.rs:1042` | la sonde du critère 6 est **trop large** : `init.rs:17` est `AgentContext.llm`, que le ticket demande de **garder**. Voir D9 |

---

## 2. Décisions

### D1 — `compact_par_spirit(app: &App, base_url, lire_env) -> String`

```rust
async fn compact_par_spirit(app: &App<'_>, base_url: &str, lire_env: SourceEnv<'_>) -> String
```

La forme de `clear_par_spirit` (`handlers.rs:137`), à une différence près et pour
une raison mesurée : **`&App` et non `&mut App`**. `/compact` ne modifie aucun
champ — ni l'affichage, ni `session_id`, ni le worker : la conversation est
recomposée côté serveur et la TUI la relira au prochain poll comme n'importe
quelle écriture de la base. Un `&mut` que rien n'utilise annoncerait dans la
signature une mutation que le corps ne fait pas, et autoriserait un appelant
futur à en glisser une sans que la revue la voie. Le `&mut App` du dispatch
reborrow en `&App` sans un mot de plus.

**L'URL de base et la source d'environnement arrivent par argument**, comme pour
les trois fonctions voisines : un client qui résout ses propres accès ne se
pilote pas depuis un test, et un chargement caché derrière le paramètre ferait
mentir la signature. L'enveloppe `handle_compact` garde la charge du `.env` et de
`spirit_url()`, exactement comme `handle_clear` (l. 112-121).

**L'ordre du corps est le contrat des critères 1 et 2, et il est mesurable :**

1. `if app.status != AgentStatus::Idle { return … }` — refus local, **avant**
   toute lecture de jeton et tout appel (D2) ;
2. `let jeton = match jeton_interne(lire_env) { Ok(j) => j, Err(refus) => return refus };`
   — le refus nomme `MIKA_INTERNAL_TOKEN` parce que c'est `jeton_interne` qui le
   nomme. **Aucune seconde lecture du jeton n'est écrite** : la lecture unique de
   V2d-4b reste unique, et `grep -rn "fn jeton_interne" crates/mika-cli/src`
   doit toujours rendre une ligne ;
3. `session_remote::compacter(base_url, &jeton, &app.agent_name).await`, puis les
   quatre bras de D4 et D5.

Le retour est `String` et non `Result` : c'est ce que le dispatch affiche, et
c'est la forme des trois commandes voisines.

### D2 — le refus « pas au repos » reste **local**, et ce n'est pas une redondance

Le serveur prend le verrou de l'agent et rend 409 — on pourrait croire que le
refus local double une décision que mika-spirit sait prendre. **La mesure dit le
contraire, et c'est le fait le plus important de ce vol.**

Tant que V1 n'est pas livré, le tour de la TUI tourne **dans le processus de la
TUI** (`chat.rs:341` et `chat.rs:468` appellent `agent::run_agent`). Le verrou
que `/api/v1/compact` tente de prendre est celui de l'agent **côté serveur** :
un tour local ne le détient pas. Un `/compact` lancé pendant qu'un tour tourne
dans la TUI obtiendrait donc un **200**, et `maybe_compact` remplacerait les
anciens messages par un résumé pendant que le tour local les relit — exactement
le mode de panne que le commentaire de `server/compact.rs:63-69` décrit, et que
le verrou ne peut pas empêcher ici parce qu'il ne voit pas ce tour.

Le refus local n'est donc pas un double juge : il est le **seul** juge de ce
fait-là, et il le restera jusqu'à V1. À écrire dans le commentaire, parce que
c'est précisément le pourquoi qu'on ne lit pas dans le code.

Conséquence : les deux refus « occupé » **disent deux choses différentes** et
gardent deux messages distincts. Le local dit « ton propre tour tourne » ; le 409
dit « un autre client tient le verrou de cet agent » (D5). Les fondre sous un
message unique enverrait l'opérateur attendre la fin de son propre tour alors
qu'un autre client est en cause, ou l'inverse.

### D3 — le seuil disparaît du client, et le compte vient du serveur

Aujourd'hui le client compare `count_messages()` à `50` écrit en dur
(`handlers.rs:270`) et affiche `50` comme dénominateur (l. 271), tandis que le
serveur **lit** `compaction::COMPACTION_THRESHOLD` (`server/compact.rs:93`). Deux
vérités sur le même seuil, dont l'une est déjà une copie figée : si la constante
bouge, le client refuse ou accepte au mauvais rang et affiche un dénominateur
faux, en silence.

Après ce vol, le client **ne compare plus rien et ne connaît plus de seuil**. Le
serveur rend déjà le fait et le compte : `ok(false, messages)` au rang sous le
seuil, `ok(true, messages)` sinon. Le message de `compacted: false` porte donc le
compte **rendu** et **aucun dénominateur** — en afficher un obligerait le client à
reconnaître un seuil qu'il vient de cesser de connaître.

`count_messages()` n'est plus appelé sur le chemin `/compact` ; il reste appelé
par `/status` (l. 444), ce qui est pourquoi il ne peut pas devenir un motif de
garde (D6).

### D4 — `compacted: true` dit **demandée**, et le test interdit les deux mots

Le contrat est déjà écrit à la porte (`session_remote.rs:72-80`) :
`CompactedMirror.compacted` veut dire « la compaction a été **demandée et n'a pas
échoué** », et jamais « une ligne `summary` a été écrite ». Les deux booléens sont
des 200 ; `true` dit que `maybe_compact` a rendu `Ok`, mais elle relit le seuil
elle-même et peut légitimement n'avoir rien fait.

Un message qui dirait « Compacted conversation » — ce que dit le code d'aujourd'hui
(l. 274) — affirmerait donc un état du système que le serveur s'est gardé
d'attester : c'est la grounding rule du dépôt, pas une préférence de rédaction.

Les deux bras de succès :

| `compacted` | message |
|---|---|
| `false` | `Nothing to compact ({messages} messages).` |
| `true` | `Compaction requested ({messages} messages before). mika-spirit does not report whether a summary was written.` |

Le critère 2 exige que le second « ne contienne ni « compacted » au passé ni
« compactée » ». La formulation retenue n'a aucune des deux sous-chaînes, et le
test l'**atteste par un `assert!` négatif** sur la minuscule, pas par relecture :
`!message.to_lowercase().contains("compacted")` et
`!message.to_lowercase().contains("compactée")`. Une rédaction future qui
reviendrait au passé rougira.

### D5 — `Busy` a son bras ; tout le reste porte le `Display`

```rust
Err(SessionRemoteIssue::Busy) => "mika-spirit is busy with this agent — another \
    client holds its turn lock. Try /compact again when it is idle.".to_string(),
Err(probleme) => format!("Compaction failed: {probleme}"),
```

Le `Display` de `Busy` dit la **cause** (« another turn holds its lock ») et non
le **remède**. Devant une TUI, c'est le remède qui décide de ce que l'opérateur
fait de la seconde qui suit : un bras dédié le dit, et dit aussi que l'occupant
n'est **pas** son propre tour — que le refus local de D2 aurait déjà intercepté.

Tout le reste — 401, 404, 500, corps illisible, spirit injoignable — passe par le
`Display` de `SessionRemoteIssue`, qui est déjà écrit pour nommer sa cause sans
en inventer une (motif mika#2331 : `status: None` dit « aucune réponse reçue » là
où un `0` se lirait comme un statut HTTP). Le client n'a rien à y ajouter.

### D6 — la garde de `handlers.rs` gagne `maybe_compact(`

```rust
const ALLOWED: &[&str] = &[];
assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");
const MOTIFS: &[(&str, &str)] = &[
    ("create_session(", "session_remote::ouvrir"),
    ("end_session(", "session_remote::clore"),
    ("maybe_compact(", "session_remote::compacter"),
];
```

Quatre propriétés sont **conservées telles quelles**, parce qu'elles sont le
livrable de la garde et non sa décoration : l'allowlist vide **et assertée vide**
(quand la garde rougit, le remède est la porte que son message nomme, jamais une
entrée) ; le témoin `async fn handle_clear(` ; le contre-témoin
`fn test_clear_singleton_does_not_end_session` ; et un `assert!` final par motif,
pour que le rouge nomme **tous** les sites à déplacer.

Le nom de la garde — `mika1727_v2d4b_…` — **ne change pas**. C'est une garde par
fichier, pas par vol : la renommer casserait la trace des trois vols qui la
citent, pour un gain nul.

Son doc-comment (l. 3020-3024) est réécrit : il dit aujourd'hui que
`maybe_compact(` « appartient à V2d-4c », ce qui est faux sur le vol **et** faux
sur le fond après ce vol.

**`count_messages(` n'est pas et ne peut pas être un motif.** Il reste appelé
légitimement par `/status` (l. 444), et un motif qui rougirait sur un appel
légitime n'aurait d'autre issue qu'une entrée d'allowlist. Le critère 3 se tient
donc autrement : le motif `maybe_compact(` ferme le chemin local, et le test T-D
atteste que le compte affiché vient du **corps servi** et non d'un comptage
local — il sert `"messages":7` sur une base qui n'a pas sept messages, de sorte
qu'un comptage local résiduel rendrait un autre nombre.

### D7 — les deux `expect(dead_code)` tombent dans le commit qui câble

L'attribut est `#[cfg_attr(not(test), expect(dead_code, …))]`, et c'est ce choix
(fait par V2d-3a au lieu d'`allow`) qui rend le moment **non négociable** : dès
que `compacter` est atteinte hors test, l'attente devient
`unfulfilled_lint_expectation` et `-D warnings` rougit. Avant le câblage, la
retirer laisse un `dead_code` nu, qui rougit aussi.

Pour `CompactedMirror`, l'attribut est posé sur l'**item**, donc il couvre ses
champs : lire un seul champ laisserait l'attente satisfaite. Le câblage lit les
**deux** (`compacted` dans le `match`, `messages` dans les deux messages), donc
les deux `expect` tombent ensemble, dans le commit 1.

Le § *État du vol* du doc de module est réécrit : les trois portes sont câblées,
il ne reste **aucun** `expect(dead_code)`, et le chiffre se **mesure** sous
`cargo clippy -p mika-cli --all-targets`. La phrase qui promettait le câblage
« quand V2d-4c câblera la dernière » part avec — elle nommait déjà le mauvais
vol. La propriété que ce paragraphe porte depuis V2d-3a est conservée : une passe
clippy verte tient le chiffre exact **dans les deux sens**.

`grep -c "expect(dead_code" crates/mika-cli/src/commands/session_remote.rs` rend
**0** (critère 5).

### D8 — `App.llm` est retiré dans le **dernier** commit, seul

Ce que le retrait touche, mesuré site par site :

| fichier | geste | ajouts |
|---|---|---|
| `tui/app.rs` | déclaration l. 578, paramètre l. 691, pose l. 730, pose `new_team` l. 825 (`dummy_provider()`), import l. 13, et la ligne `dummy_provider()` du constructeur de test l. 2361 | 0 |
| `commands/chat.rs` | l'argument `worker._ctx.llm.clone()` d'`App::new` (l. 663) et les deux affectations `app.llm = …` (l. 935, 1042) | 0 |
| `tui/commands/handlers.rs` | `NoopLlmProvider` (l. 1474-1504), `let llm: …` (l. 1530), l'argument (l. 1541), et l'import l. 1471 **réduit** à `ProviderKind` | 1 |
| `tui/input.rs` | `NoopLlmProvider` (l. 1127-1154), `let llm: …` (l. 1178), l'argument, et l'import l. 1121 **réduit** à `ProviderKind` | 1 |

Les deux imports se **réduisent** plutôt que disparaissent : `ProviderKind` du
même `use` reste utilisé par chaque `test_app()`, et `Arc` par
`SkillRegistry::empty()`. Le `#[allow(clippy::too_many_arguments)]` d'`App::new`
reste nécessaire (13 paramètres pour un seuil de 7) : ce vol n'ajoute ni ne
retire aucun `allow`.

`worker._ctx.llm` **reste** : le worker s'en sert (`chat.rs:310` et `chat.rs:482`),
et c'est `AgentContext.llm` dans `init.rs`, un autre champ sur un autre type.

**Pourquoi seul, et pourquoi en dernier.** Le retrait est en retraits quasi
purs — deux lignes ajoutées sur ~75 retirées — et ne change aucun comportement.
L'isoler dans le dernier commit a une valeur : si la CI y rougit sur un site que
la mesure a manqué, les quatre livrables fonctionnels sont déjà verts et commités,
et le vol reste livrable. C'est ce que le ticket demande, et la raison tient.

### D9 — la sonde du critère 6 est trop large, et le plan écrit la juste

Le critère 6 dit : « `grep -rn "app\.llm\|pub llm:" crates/mika-cli/src/` ne rend
rien ». **Cette sonde ne peut pas rendre zéro**, et pas à cause de ce vol :
`init.rs:17` porte `pub llm: Arc<dyn LlmProvider>,` — c'est `AgentContext.llm`,
dont le § *Périmètre* du même ticket dit explicitement « *`worker._ctx.llm`
reste : le worker s'en sert* ». La sonde et le périmètre du ticket se
contredisent ; le fait visé, lui, est sans ambiguïté : **`App.llm` n'existe
plus**.

Ce n'est **pas** la clause de halte du ticket. Celle-ci se déclenche quand « *un
critère ne tient pas sans toucher un fichier exclu* » : ici le critère tient
entièrement, `init.rs` n'est pas touché, et c'est l'instrument qui est trop
large — pas le périmètre qui est trop étroit. La même classe de défaut a déjà
été nommée dans ce ticket au vol précédent (le critère 9 de V2d-4c nommait un
seul constructeur de test là où la mesure en trouvait trois). On ne s'arrête pas
sur un défaut de sonde ; on le nomme et on écrit la mesure qui porte le fait :

```
grep -rn "app\.llm" crates/mika-cli/src/            # → rien
grep -n "llm" crates/mika-cli/src/tui/app.rs        # → rien
grep -rn "NoopLlmProvider" crates/mika-cli/src/     # → rien
```

Les trois sont au § 6, et la PR dit en une phrase pourquoi la sonde du critère a
été corrigée plutôt que suivie.

### D10 — la doc dit `/compact`, et dit que V2d-4 est fini

`crates/mika-cli/CLAUDE.md`, section TUI, gagne une sous-section
`### /compact passe par mika-spirit (mika#1727, vol V2d-4d)` : ce que fait la
commande, **pourquoi le seuil n'est plus côté client** (D3), **pourquoi
`compacted: true` veut dire « demandée »** (D4), et pourquoi le refus « pas au
repos » reste local tant que V1 n'est pas livré (D2).

Trois phrases existantes sont réconciliées parce qu'elles deviennent fausses :

- l. 574-577 (la « phrase de la l. 574 » du ticket) : « `maybe_compact(` n'est
  pas un motif : il part au vol suivant… » → la garde refuse désormais les
  **trois** motifs ;
- l. 641-642 : « Ce vol ne câble pas `compacter` : `/compact` compacte encore en
  local, et les deux `expect(dead_code)` de `session_remote.rs` restent
  **deux**. » → remplacée par le fait, et par **zéro** ;
- la ligne de slash command de `/clear` (l. 645) est suivie d'une ligne `/compact`
  neuve, la commande n'en ayant aucune aujourd'hui.

La section dit enfin que **V2d-4 est fini** : le démarrage (4a), `/clear` (4b),
les trois sorties (4c) et `/compact` (4d) passent tous par mika-spirit, et la
suite du ticket est V3 puis V4.

### D11 — aucun ticket n'est ouvert

Gel jusqu'au 01/12. Ce qui reste va dans le corps de la PR, sous
`Tracked in: senara-solutions/mika#1727`. Pas de sous-ticket, pas de ticket de
suivi.

---

## 3. Livrables, un par un

1. **`compact_par_spirit`** dans `tui/commands/handlers.rs`, à côté de
   `clear_par_spirit` : signature et ordre de corps de D1, bras de D4 et D5.
2. **`handle_compact` devenu enveloppe** : `.env` + `spirit_url()` + délégation,
   la forme de `handle_clear` (l. 112-121). Le seuil `50`, la comparaison et
   l'appel à `count_messages()` disparaissent du chemin `/compact`.
3. **Les deux `expect(dead_code)` de `session_remote.rs`** retirés, et le § *État
   du vol* du doc de module réécrit (D7).
4. **La garde** `mika1727_v2d4b_la_tui_ne_clot_plus_la_base_en_local` : motif
   `maybe_compact(` → `session_remote::compacter`, doc-comment réécrit (D6).
5. **Sept tests** (§ 5) dans le `mod tests` de `handlers.rs`, avec les fixtures
   qui y sont déjà.
6. **La doc** : `crates/mika-cli/CLAUDE.md` selon D10.
7. **Le retrait d'`App.llm`** selon D8, dans le dernier commit, seul.

---

## 4. Ordre des commits

Quatre commits. Le pilote mesure **après chacun** :
`git diff --numstat origin/main...HEAD`, plan exclu.

| # | contenu | fichiers | critères |
|---|---|---|---|
| 1 | Le motif `maybe_compact(` ajouté à la garde **seul**, le rouge-avant montré (il doit nommer la l. 273, et elle seule), puis `compact_par_spirit` + l'enveloppe `handle_compact` + les deux `expect` retirés + le doc du module + le doc-comment de la garde, jusqu'au vert | `handlers.rs`, `session_remote.rs` | 3, 4, 5 |
| 2 | Les sept tests de `/compact` | `handlers.rs` | 1, 2 |
| 3 | La doc | `crates/mika-cli/CLAUDE.md` | 5 (la part « la doc le dit »), 7 |
| 4 | Le retrait d'`App.llm`, **seul** | `tui/app.rs`, `commands/chat.rs`, `handlers.rs`, `tui/input.rs` | 6 |

Le commit 1 porte la garde **avec** son test — la garde *est* un test, et son
rouge-avant est la preuve qu'elle mesure quelque chose. Il porte aussi les deux
`expect` et le câblage, qui sont indissociables (D7) : les séparer ferait rougir
la CI sur le commit intermédiaire. Les sept tests de comportement suivent au
commit 2 : « chaque garde arrive avec ses tests, dans le même commit » est tenu
par le commit 1, et des commits plus petits rendent la halte plus fine, comme le
ticket le demande. **Les tests ne se reportent jamais à un vol suivant.**

**Halte du pilote sur la borne.** Après chaque commit, si le total dépasse **500**
lignes ajoutées : arrêt. Pas de commit suivant, pas de PR. Dernier message :
« HALTE borne : N lignes pour 500 », avec le compte par commit. À 260 lignes
attendues, cette halte ne devrait pas se déclencher ; si elle le fait, c'est que
l'estimation du § *Taille estimée* est fausse, et le pilote s'arrête **plutôt que
de raboter des commentaires** — le ticket l'interdit nommément.

---

## 5. Jeu de tests

Tous dans le `mod tests` de `tui/commands/handlers.rs`, avec `fake_spirit`,
`RequeteRecue`, `Mutex`, `Arc`, `SourceEnv`, `avec_jeton()` et `test_app()`
**déjà présents** : zéro import à ajouter. Le banc est **emprunté**, jamais
recopié — un second témoin de socket ouvrirait une seconde vérité sur ce que le
serveur reçoit, libre d'en diverger. Les appels se comptent par `vues.len()`,
`RequeteRecue` ne dérivant pas `Debug`.

Deux aides locales, sur le modèle de `par_le_banc` (l. 1635) :
`const ROUTE_COMPACT: &str = "/api/v1/compact";` et
`async fn par_le_banc_compact(app, reponses, lire_env) -> (String, Arc<Mutex<Vec<RequeteRecue>>>)`.

| id | nom | ce qu'il mesure | critère |
|---|---|---|---|
| T-A | `mika1727_v2d4d_un_agent_occupe_refuse_sans_aucun_appel` | `app.status = AgentStatus::Busy`, banc monté et jeton posé → le message dit l'occupation, `vues.len() == 0`. Le banc **monté** est le montage : sans lui, « aucun appel » ne se distingue pas de « aucun serveur » | 1 |
| T-B | `mika1727_v2d4d_sans_jeton_compact_refuse_avant_tout_appel_en_nommant_la_variable` | au repos, source d'environnement **vide** → message contenant `MIKA_INTERNAL_TOKEN`, `vues.len() == 0`. **Contrôle positif obligatoire** : le même montage avec `avec_jeton` doit atteindre la socket, sinon un refus tombé pour toute autre raison rendrait ce test vert en mesurant autre chose | 1 |
| T-C | `mika1727_v2d4d_compact_poste_sur_la_route_de_compaction_avec_son_jeton_et_son_agent` | sur la socket : `POST /api/v1/compact`, `Authorization: Bearer jeton-interne`, et `agent` **désérialisé** égal à `app.agent_name` (le serveur lit un champ, pas un ordre de clés) | 2 |
| T-D | `mika1727_v2d4d_rien_a_compacter_porte_le_compte_rendu_et_aucun_seuil` | corps `{"compacted":false,"messages":7}` en 200 → le message dit qu'il n'y a rien à compacter et porte `7`. **Et ne porte pas `50`** : c'est la moitié exécutable du critère 3, et `7` est choisi parce qu'il ne peut pas venir d'un comptage local de la base de `test_app()` | 2, 3 |
| T-E | `mika1727_v2d4d_une_compaction_est_demandee_jamais_affirmee_faite` | corps `{"compacted":true,"messages":64}` → le message porte `64`, dit « requested », et `!message.to_lowercase().contains("compacted")` **et** `!…contains("compactée")`. La grounding rule attestée par assertion, pas par relecture | 2 |
| T-F | `mika1727_v2d4d_un_409_dit_loccupation_et_le_nouvel_essai` | banc répondant `409` → le message dit que l'agent est occupé et qu'on peut réessayer, et la socket a bien été touchée (`vues.len() == 1`) | 2 |
| T-G | `mika1727_v2d4d_une_autre_erreur_porte_le_display_du_probleme` | banc répondant `500` avec un corps → le message porte le `Display` de `SessionRemoteIssue` (il contient `500`). Un 500 plutôt qu'un `DeadEndpoint` : aucun critère n'exige un point injoignable ici, c'est le montage déjà employé l. 2979-3000, et il ne coûte aucun import | 2 |
| T-H | `mika1727_v2d4b_la_tui_ne_clot_plus_la_base_en_local` (**existante, étendue**) | trois motifs, allowlist vide **et assertée vide**, témoin, contre-témoin | 4 |

T-A et T-B sont « un test par cas » du critère 1 ; T-C à T-G celui du critère 2.
**Aucun test neuf ne contiendra la chaîne `maybe_compact(`** — la fonction
appelée est `compacter` et les noms sont en français : le motif n'a pas de raison
d'apparaître dans la moitié test. S'il y était introduit un jour, le
contre-témoin de T-H dirait lequel des deux défauts s'est produit.

---

## 6. Definition of Done

- [ ] `compact_par_spirit(app: &App<'_>, base_url: &str, lire_env: SourceEnv<'_>) -> String`
      existe dans `handlers.rs`, et son corps refuse dans l'ordre :
      pas-au-repos → jeton → appel.
- [ ] `handle_compact` ne fait plus que charger le `.env`, lire `spirit_url()` et
      déléguer, comme `handle_clear`.
- [ ] Les quatre bras sont écrits : `compacted: false`, `compacted: true`
      (« requested », compte d'avant, ni « compacted » ni « compactée »), `Busy`
      (occupation + nouvel essai), et le `Display` de `SessionRemoteIssue`.
- [ ] Le commentaire de D2 dit pourquoi le refus « pas au repos » reste local
      tant que le tour de la TUI ne passe pas par spirit.
- [ ] `grep -n "50" crates/mika-cli/src/tui/commands/handlers.rs` ne rend plus
      les l. 270-271 ; aucun seuil de compaction n'est écrit côté client ;
      `count_messages()` n'est plus appelé sur le chemin `/compact` (il reste
      appelé par `/status`).
- [ ] La garde refuse `create_session(`, `end_session(` **et** `maybe_compact(` ;
      `ALLOWED` est vide et `assert!(ALLOWED.is_empty(), …)` est présent ; témoin
      et contre-témoin présents ; doc-comment réécrit.
- [ ] Le rouge-avant de `maybe_compact(` a été montré sur le code de `main`,
      nommant la l. 273 et elle seule, et la sortie est dans le corps de la PR.
- [ ] Les sept tests T-A à T-G passent. T-B porte son contrôle positif.
- [ ] `cargo test -p mika-cli` vert.
- [ ] `cargo clippy -p mika-cli --all-targets` vert, **sans `allow` ni `expect`
      ajouté**, et **sans attente insatisfaite**.
- [ ] `cargo fmt --check` vert.
- [ ] `grep -c "expect(dead_code" crates/mika-cli/src/commands/session_remote.rs`
      rend **0**, et le § *État du vol* du doc de module le dit, en nommant
      `cargo clippy -p mika-cli --all-targets` comme la mesure.
- [ ] `grep -rn "fn jeton_interne" crates/mika-cli/src` rend **une** ligne.
- [ ] `grep -rn "app\.llm" crates/mika-cli/src/` ne rend rien.
- [ ] `grep -n "llm" crates/mika-cli/src/tui/app.rs` ne rend rien.
- [ ] `grep -rn "NoopLlmProvider" crates/mika-cli/src/` ne rend rien.
- [ ] `App::new` n'a plus de paramètre `llm` ; `App::new_team` ne pose plus
      `dummy_provider()` ; les trois constructeurs de test sont à jour.
- [ ] Le retrait d'`App.llm` est le **dernier** commit de la PR, et il ne porte
      que ce retrait.
- [ ] `crates/mika-cli/CLAUDE.md` décrit `/compact` sur spirit, pourquoi le seuil
      n'est plus côté client, pourquoi `compacted: true` veut dire « demandée »,
      et dit que V2d-4 est fini ; les trois phrases périmées sont réconciliées.
- [ ] `git diff --name-only origin/main...HEAD` (plan exclu) ne rend que
      `crates/mika-cli/src/tui/commands/handlers.rs`,
      `crates/mika-cli/src/commands/session_remote.rs`,
      `crates/mika-cli/CLAUDE.md`, `crates/mika-cli/src/tui/app.rs`,
      `crates/mika-cli/src/commands/chat.rs` et
      `crates/mika-cli/src/tui/input.rs`.
- [ ] `bash scripts/verify-egress-manifest.sh --report` vert, **aucune entrée
      modifiée** (l'entrée `cli-spirit-sessions` désigne toujours
      `session_remote.rs`). Rouge ⇒ le pilote s'arrête et le dit.
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert.
- [ ] `git diff --numstat origin/main...HEAD` (plan exclu) ≤ **500** lignes
      ajoutées.
- [ ] Le corps de la PR porte `Refs mika#1727` et **aucun** `Closes`, `Fixes` ni
      `Resolves` ; `gh pr view <N> --json closingIssuesReferences` rend `[]`.
- [ ] Le corps de la PR porte
      `Code review: delegated to mika-qa (mika#2694)`, la correction de sonde de
      D9 en une phrase, et `Tracked in: senara-solutions/mika#1727` pour la suite.
- [ ] Aucun ticket ouvert.

---

## 7. Acceptance criteria

Les huit critères du ticket, transcrits **verbatim**, avec pour chacun ce qui le
tient.

1. **« `/compact` sur un agent pas au repos refuse sans appel, et le banc ne
   reçoit rien. Sans jeton, il refuse avant tout appel, avec un message qui nomme
   `MIKA_INTERNAL_TOKEN`. Un test par cas. »** — Tenu : D1 (ordre du corps), T-A,
   T-B. T-A monte le banc **et** pose le jeton, pour que « aucun appel » se
   distingue de « aucun serveur » ; T-B porte son contrôle positif.

2. **« `/compact` envoie `POST {base}/api/v1/compact`, avec
   `Authorization: Bearer <jeton>` et `agent`, lus sur la socket du banc. Sur
   `compacted: false`, le message dit qu'il n'y a rien à compacter, avec le compte
   rendu. Sur `compacted: true`, le message dit que la compaction a été demandée,
   avec le compte d'avant, et ne contient ni « compacted » au passé ni
   « compactée ». Sur un 409, le message dit que l'agent est occupé. Sur une autre
   erreur, il porte le `Display` de `SessionRemoteIssue`. Un test par cas. »** —
   Tenu : D4, D5, T-C, T-D, T-E, T-F, T-G. Les deux interdictions de mots sont des
   `assert!` négatifs sur la minuscule (T-E), pas une relecture.

3. **« Aucun seuil de compaction n'est écrit dans `handlers.rs` : la moitié
   production ne compare plus `count_messages()` à une constante pour
   `/compact`. »** — Tenu : D3 retire les deux `50` (l. 270 et l. 271) et l'appel
   à `count_messages()` du chemin `/compact`. Attesté par deux mesures
   complémentaires : la garde (motif `maybe_compact(`, qui ferme le chemin local)
   et T-D, qui sert `"messages":7` — un nombre qu'un comptage local résiduel ne
   pourrait pas rendre. `count_messages(` ne peut **pas** être un motif de garde :
   `/status` l'appelle légitimement (l. 444), et un motif qui rougirait dessus
   n'aurait d'autre issue qu'une entrée d'allowlist.

4. **« La garde de `handlers.rs` refuse `create_session(`, `end_session(` et
   `maybe_compact(`. Allowlist vide et assertée vide, témoin présent. Le pilote
   montre le rouge-avant du motif neuf sur le code de main, puis le vert. »** —
   Tenu : D6, T-H. Le rouge-avant est gratuit — il suffit d'ajouter le motif avant
   de câbler — et doit nommer la l. 273, et elle seule. C'est la première chose du
   commit 1, et la sortie va dans le corps de la PR.

5. **« Le pilote lance `cargo clippy -p mika-cli --all-targets`. Le résultat est
   vert, sans `allow`, et sans attente insatisfaite. `grep -c "expect(dead_code"
   crates/mika-cli/src/commands/session_remote.rs` rend 0, et la doc du module le
   dit. »** — Tenu : D7. Les deux `expect` tombent dans le commit qui câble, ce
   qui est **forcé** et non choisi (`-D warnings` rougit sur l'un comme sur
   l'autre bord). Aucun `allow` n'est ajouté ; celui d'`App::new`
   (`too_many_arguments`) reste nécessaire à 13 paramètres et n'est pas touché.

6. **« `App.llm` n'existe plus : `grep -rn "app\.llm\|pub llm:"
   crates/mika-cli/src/` ne rend rien. `App::new` n'a plus de paramètre `llm`, et
   les deux `NoopLlmProvider` de test sont partis. Ce retrait est le dernier
   commit de la PR. »** — Tenu **sur le fait, avec une sonde corrigée** (D9). Le
   fait — `App.llm` n'existe plus, `App::new` n'a plus le paramètre, les deux
   `NoopLlmProvider` sont partis, et le retrait est le dernier commit — est tenu
   en entier par D8. La sonde écrite dans le critère ne peut pas rendre zéro :
   `init.rs:17` porte `pub llm:` pour `AgentContext.llm`, que le § *Périmètre* du
   même ticket demande de **garder**. Les trois mesures du § 6 portent le fait
   sans ce faux positif. Ce n'est pas la clause de halte : `init.rs` n'est pas
   touché, et c'est l'instrument qui est trop large, pas le périmètre qui est trop
   étroit. La PR le dit en une phrase.

7. **« Seuls `tui/commands/handlers.rs`, `commands/session_remote.rs`,
   `crates/mika-cli/CLAUDE.md` et, pour le seul retrait d'`App.llm`, `tui/app.rs`,
   `commands/chat.rs` et `tui/input.rs` changent. Le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il le
   trouve rouge, il s'arrête et le dit. »** — Tenu : les six fichiers, et aucun
   autre. `handlers.rs` appelle une porte et ne construit aucun client vers les
   routes de session : l'entrée `cli-spirit-sessions` désigne toujours
   `session_remote.rs`, et le manifeste ne change pas. S'il faut le changer, le
   pilote s'arrête et le dit.

8. **« La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
   compris si le gabarit de reprise l'ouvre. »** — Tenu. Si le label `wip-rescue`
   ouvre la PR, son gabarit écrit `Closes #N` par défaut (corrigé à la main sur
   #3077 et #3081) : le pilote le corrige **avant toute revue**, puis vérifie que
   `gh pr view <N> --json closingIssuesReferences` rend `[]`.

**Et la clause de halte du ticket :** « *Si le groom juge qu'un critère ne tient
pas sans toucher un fichier exclu ci-dessus, il s'arrête et le dit, au lieu
d'élargir.* » Elle n'est **pas déclenchée**. Les huit critères se tiennent dans
les six fichiers que le critère 7 énumère. Le seul écart rencontré est un défaut
de **sonde** au critère 6, qui ne demande aucun fichier de plus (D9).

---

## Fire-Disposition

Ce vol livre un détecteur : la garde
`mika1727_v2d4b_la_tui_ne_clot_plus_la_base_en_local` gagne le motif
`maybe_compact(`. Section obligatoire (mika#2306).

**Option retenue : (a) — allowlist nommée, livrée à zéro entrée, et assertée
vide.**

```rust
const ALLOWED: &[&str] = &[];
assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");
```

Les quatre propriétés de mika#1574 :

1. **Le détecteur est livré désarmé-par-vacuité, pas désarmé-par-exception.**
   `ALLOWED` est vide parce qu'il n'y a **aucun** site légitime : après le
   livrable 2, la moitié production de `handlers.rs` ne contient plus
   `maybe_compact(`. Le rouge-avant du commit 1 (qui doit nommer la l. 273, et
   elle seule) est la preuve que le détecteur mesure quelque chose ; le vert
   d'après, la preuve qu'il n'y a rien à excepter. **Le motif ne précède jamais
   son câblage** : un détecteur armé sur du code que rien ne remplace ne laisse
   d'autre issue qu'une entrée d'allowlist, ce que cette section existe pour
   refuser — c'est la raison pour laquelle ce motif a été reporté de V2d-4b à ce
   vol, et pas l'inverse.
2. **Le remède nommé est une porte, jamais une entrée.** Le message d'échec porte
   `remède : session_remote::compacter` pour chaque ligne fautive. Un appelant
   futur se route par la porte existante (doctrine mika#2201).
3. **L'allowlist ne peut pas cesser d'être vide en silence.**
   `assert!(ALLOWED.is_empty(), …)` rougit à la première entrée ajoutée, avant
   même le balayage : une allowlist qui se remplit est le faux-vert que cette
   forme existe pour refuser (motif mika#2309 / mika#2457).
4. **Le détecteur ne peut pas être vert par cécité.** Témoin
   (`async fn handle_clear(` — la garde a lu la production) **et** contre-témoin
   (`fn test_clear_singleton_does_not_end_session` absent — la garde a masqué la
   moitié test). `production_of()` masque les régions `#[cfg(test)]` sans jamais
   tronquer et laisse une région infermable **visible** exprès
   (`SliceReport::unclosed`, mika#2398) : les deux témoins distinguent les deux
   pannes, là où un seul `assert!` les confondrait.

**`count_messages(` n'est délibérément pas un motif.** `/status` l'appelle
légitimement (`handlers.rs:444`) : un motif qui rougirait dessus n'aurait d'autre
issue qu'une entrée d'allowlist — la Fire-Disposition de l'option (a) refuse
d'armer un détecteur dont le seul remède serait une exception. Le critère 3 se
tient par `maybe_compact(` et par T-D.

**`expect(dead_code)` : les deux tombent, et aucun n'est ajouté.** La
Fire-Disposition de V2d-3a avait choisi `expect` plutôt qu'`allow` précisément
pour que le build **dise** le moment du câblage : l'attente devient
`unfulfilled_lint_expectation` dès que la porte est atteinte, et
`cargo clippy --all-targets --all-features -- -D warnings` (`ci.yml:76`) la
refuse. Ce vol est ce moment. Le compte passe de **2** à **0**, mesuré sous
`cargo clippy -p mika-cli --all-targets` et non deviné, et le doc de module
conserve la phrase qui dit que la mesure tient le chiffre exact **dans les deux
sens** — un `expect` de trop serait « unfulfilled », un de moins laisserait un
`dead_code` nu.

---

## 8. Ce que ce vol ne fait pas

- **`commands/ask.rs` n'est pas la TUI** et n'est pas touché, le ticket le dit
  nommément. `rewind_remote.rs`, `dashboard.rs`, `crates/mika-agent/` et
  `crates/mika-a2a/` restent intacts. `Tracked in: senara-solutions/mika#1727`.
- **`worker._ctx.llm` reste**, et avec lui `AgentContext.llm` (`init.rs:17`) : le
  worker s'en sert (`chat.rs:310` et `chat.rs:482`). C'est un autre champ, sur un
  autre type, et le ticket demande de le garder.
- **Le refus « pas au repos » reste côté client**, et il le restera jusqu'à V1
  (D2). Le jour où le tour de la TUI passe par mika-spirit, le verrou de l'agent
  verra ce tour et le 409 suffira : ce refus local redeviendra une redondance, et
  c'est à V1 de le retirer, pas à ce vol de l'anticiper.
- **Les deux `tracing::warn!` du voisin ne sont pas touchés**, et aucun n'est
  ajouté : `/compact` rend son message à l'écran, qui est le destinataire.
- **La compaction d'après-tour n'est pas touchée.** `mika_agent::compaction` reste
  appelée côté serveur ; ce vol ne retire que l'appel **local** du chemin
  `/compact`.
- **Aucun fichier du plan de contrôle** : ni `.github/workflows/`, ni `.claude/`,
  ni `skills/bundled/`, ni `crates/mika-agent/src/well_known_agents.rs`, ni
  `.mika/`, ni aucun chemin de CODEOWNERS. `docs/egress/`, `docs/openapi/` et
  `docs/slash-commands.md` ne sont pas touchés.
- **Aucun ticket n'est ouvert** (gel jusqu'au 01/12), pas même de suivi.
- **V3 et V4 ne sont pas redécoupés.** #1727 se ferme au merge de V4, et de là
  seulement — donc **pas** au merge de celui-ci, bien qu'il close V2d-4.

---

## Taille estimée

| poste | lignes ajoutées |
|---|---|
| `compact_par_spirit` + son doc-comment + les commentaires de D2/D3/D4 (`handlers.rs`) | 48 |
| `handle_compact` devenu enveloppe (`handlers.rs`) | 12 |
| La garde : motif `maybe_compact(` + doc-comment réécrit | 7 |
| Les deux `expect` retirés + § *État du vol* réécrit (`session_remote.rs`) | 10 |
| Les sept tests T-A…T-G + deux aides locales (`ROUTE_COMPACT`, `par_le_banc_compact`) | 155 |
| Retrait d'`App.llm` : les deux imports réduits (~75 lignes **retirées**) | 2 |
| **sous-total hors `docs/` et hors `CLAUDE.md`** | **234** |
| `crates/mika-cli/CLAUDE.md` (D10) | 30 |
| **total dans l'unité du ticket** (`git diff --numstat`, plan exclu) | **264** |

Total estimé : 234 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités, et laquelle mord.** Le total ci-dessus est écrit dans l'unité
de mika#2636 — **lignes de code ajoutées, hors `docs/` et hors `CLAUDE.md`** —,
celle que la borne par vol (429) compare. La borne du ticket (500) compte
`git diff --numstat origin/main...HEAD` plan exclu, **doc et `CLAUDE.md`
compris** : 264. Les deux sont écrites plutôt que l'une devinée, parce que c'est
le seul moyen pour que la halte du pilote et le jugement de l'architecte portent
sur le même nombre. La borne par PR (1000) ne mord pas.

Marge sur la borne par vol : **195 lignes (45 %)**. Marge sur la borne du ticket
(500, doc comprise) : **236 lignes**.

Cette marge est **mesurée, pas prudente**, et son calibrage vient des trois vols
jumeaux :

| vol | estimé au plan | réel hors `CLAUDE.md` | écart |
|---|---|---|---|
| V2d-4a (#3088) | — | 462 | — |
| V2d-4b (#3089) | 392 | 496 | +27 % |
| V2d-4c (#3090) | 200 | à relire sur le merge | — |

Appliqué à 234, un écart de +27 % porterait le réel à ~297, et le total du ticket
à ~335 : toujours sous les deux bornes. Le poste qui porte le risque est le même
que pour les deux vols précédents — les tests, 66 % de l'estimation — et il est
irréductible : `/compact` n'a **aucun** test aujourd'hui, et le ticket interdit
d'en reporter un. Si le réel débordait quand même, la halte du § 4 s'applique, et
le pilote s'arrête sans raboter les commentaires qui portent un pourquoi.

L'estimation est enfin **cohérente avec une mesure indépendante** : le plan de
V2d-4c, qui a coupé ce périmètre hors de son vol, l'annonçait à **254** lignes.
Les deux chiffres ont été établis séparément, à trois semaines de distance, et
ils s'accordent à 8 % près.
