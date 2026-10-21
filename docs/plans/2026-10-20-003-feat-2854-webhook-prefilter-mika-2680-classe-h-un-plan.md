# mika#2854 — Classe (h) de mika#2680 : un tour léger qui trie (vol 1/2)

**Ticket :** senara-solutions/mika#2854
**Amont :** mika#2680 (la classe (h) et son chiffrage), mika#2675 (le pré-filtre)
**Branche :** `feat/2854/webhook-prefilter-mika-2680-classe-h-un`
**État de référence :** `main` à `c5e64079`

---

## Contexte

La classe (h) est le tour qu'ouvre un `issue_comment` ou un `issues` `closed` sur un
ticket **sans tâche de boucle**. mika#2680 l'a laissée dormante derrière une question
posée à l'opérateur le 2026-10-08 : « que doit faire ce tour ? ». Vincent y a répondu le
2026-10-20, sur bearing Prime, par un commentaire sur #2680 : **option (c), un tour léger
qui trie**. Ce ticket porte le vol ; #2680 garde (g′).

Trois faits mesurés fixent le cadre.

| fait | valeur | source |
|---|---|---|
| volume total de (h) | 1,30 M tokens | corps de mika#2680 |
| cellule `mika-dev × issue_comment`, fenêtre 2026-10-19T06:02Z–16:30Z | 13 tours, 1 007 748 tokens d'entrée, release `not_classified` | mesure 3, corps de #2680 § « Vol 6 » — **pas sur `main`**, #2833 fermée sans merge |
| coût moyen d'un tour de cette cellule | 77 519 tokens d'entrée | 1 007 748 / 13 |

Ce dernier nombre est l'argument économique du vol : **l'économie vient du prompt, pas du
modèle.** Un tour complet paie son prompt système, sa mémoire, sa fenêtre de conversation
et son tableau d'outils — 77 k tokens pour lire trois lignes de commentaire. Un triage
paie le commentaire plus ~300 tokens de consigne. Sur la moitié notée, l'économie est de
l'ordre de 98 % ; sur la moitié escaladée, le surcoût est de l'ordre de 2 %.

### L'état sur `main`

- `crates/mika-agent/src/server/webhook_prefilter.rs` (2 753 l) porte déjà les deux
  moitiés de (h) dans sa **taxonomie de registre** : `TriggerClass::IssueComment`
  (« 79 % de la fenêtre mesurée le 2026-10-08 à elle seule ») et
  `TriggerClass::IssueClosed` (« Moitié de la classe (h) du ticket »). Aucune classe de
  **filtre** ne les écarte : `classify()` tombe en `None` pour les deux, donc elles
  sortent en `Gate::Charged(Release::NotClassified)`.
- Le module explique pourquoi (e) a été acceptée « quand la classe (h) du ticket le
  refuse » : un `issue_comment` porte du texte humain neuf, là où une fusion de PR est
  rare et entièrement tracée par `upstream_close_handler`. Ce vol ne contredit pas cet
  argument, il le **paie** : c'est précisément parce que le texte est neuf que le tour
  léger le fait **lire** à un brin de LLM au lieu de l'écarter en aveugle.
- Le site d'appel unique est `handlers.rs:2017–2067`, après tous les handlers structurels
  et avant `run_agent`. Les handlers qui tournent avant la porte — `verdict_handler`,
  `ci_failure_handler`, `ready_label_handler`, `milestone_context_handler`,
  `draft_pr_opened_handler`, `upstream_close_handler` — ne perdent donc rien, exactement
  comme pour (e).
- `MIKA_WEBHOOK_PREFILTER` désarme déjà le tout en amont ; chaque tour écarté écrit sa
  classe dans `skipped_by_class`.

---

## Ce que le vol établit avant d'écrire

Le ticket demande trois choses établies avant la première ligne. Elles le sont ci-dessous,
et chacune est une décision, pas une intention.

### D1 — Où se branche le triage : **dans le chemin du pré-filtre**, comme sixième décideur de `decide()`

Trois raisons, dont deux sont structurelles.

1. **Les handlers structurels tournent déjà avant la porte.** Le site d'appel est après
   eux (`handlers.rs:2017`), donc brancher le triage dans la porte ne leur retire rien —
   c'est le même argument que (e), et il est vérifiable à la lecture du site.
2. **AC5 l'exige.** `webhook_prefilter_skipped` a un écrivain unique déclaré en
   majuscules dans le module (`SOLE WRITER: skip_turn`), et c'est cette ligne qui alimente
   `skipped_by_class`. Un triage branché *après* la porte devrait écrire sa propre ligne
   d'audit, donc créer un second écrivain du même nom, donc rendre les deux populations
   non soustractibles (classe mika#2205). Branché *dans* la porte, la ligne d'audit et la
   classe sortent de l'écrivain existant, inchangé.
3. **L'escalade a déjà son canal.** `Gate::Charged(Release)` est lu par
   `emit_turn_charged`, qui écrit `webhook_turn_charged` avec son champ `release`. Une
   escalade de triage est un `Release` de plus : les deux branches du triage deviennent
   lisibles sur le registre existant, sans nouvelle sonde.

**Une honnêteté de taxonomie, pas un contournement.** Le doc-comment de `PrefilterClass`
dit aujourd'hui qu'une classe nomme « un événement écarté **sans LLM** ». La classe (h)
est écartée **avec** un brin de LLM de triage. Ce vol **amende cette phrase** — une ligne,
nommant le ticket — au lieu de laisser le type mentir sur son propre contenu.

### D2 — Quelles identités comptent comme « opérateur » : **aucune, et c'est la réponse**

Le ticket pose la contrainte et elle est tenue à la lettre : *le seuil de sûreté ne doit
pas dépendre de l'auteur*. Donc **pas d'allowlist d'auteurs, nulle part** — ni dans le
prompt, ni dans le code. Le triage juge le **contenu**.

- **Pourquoi** : un ordre peut venir d'un relais — samidarko, un orchestrateur, un
  humain qui recopie une décision prise ailleurs. Une allowlist aurait dégradé en note
  exactement les ordres qui passent par un relais, c'est-à-dire la population la plus
  susceptible de porter une décision déjà prise.
- **Conséquence assumée** : un commentaire d'un tiers qui *ressemble* à une décision
  escalade vers le tour complet. C'est le bon sens de l'erreur — le fail-open est vers le
  tour complet, jamais vers la note.
- **La ligne `by @auteur` reste dans le texte servi.** La retirer serait une seconde
  décision, non demandée, et elle appauvrirait le texte que le tour complet lira ensuite.
  Le prompt interdit de s'en servir, et **c'est asserté** par un couple de cas identiques
  à l'auteur près (§ Verification Contract, V5).

### D3 — Le modèle et le budget : le provider de l'agent, 160 tokens de sortie, 20 s, aucun outil

- **Modèle** : `AgentState.llm: Arc<dyn LlmProvider>` (`server/state.rs:57`) — celui que
  l'agent fait déjà tourner, déjà résolu, déjà budgeté, déjà attesté par
  `mika agents budget`.
- **Refus argumenté : pas de `MIKA_WEBHOOK_TRIAGE_MODEL` dans ce vol.** Un second axe de
  modèle ouvre une porte de calibration (mika#1190 : pas de swap sans
  `make calibrate-<role>` vert) que ce vol ne paie pas, et il n'achèterait rien : l'économie
  vient du **prompt** (77 519 → ~1 500 tokens d'entrée), pas du tarif par token. Si une
  mesure future montre que le triage est le poste dominant, le knob est un vol d'une
  dizaine de lignes — mais il se justifiera alors par un chiffre, pas par une intuition.
- **Budget** : `max_tokens: 160`. Le verdict tient en une ligne (§ D5).
- **Borne** : `TRIAGE_TIMEOUT_SECS: u64 = 20`, via `tokio::time::timeout`, du même ordre
  que `PR_STATE_TIMEOUT_SECS = 15` déjà dans le module. Expiration ⇒ fail-open (AC3).
- **Aucun outil** : `tools: None` dans la `LlmRequest`. C'est AC6, et c'est asserté **sur
  la requête construite**, c'est-à-dire sur le tableau servi, pas sur une intention
  déclarée en prose.

---

## Décisions d'implémentation

### D4 — Le candidat, et ce qu'il refuse

`classify()` reste **pure** et gagne un bras :

```rust
Candidate::HumanTextNoTask { issue_url: String, half: TriageHalf }
enum TriageHalf { Comment, IssueClosed }
```

Le bras extrait l'URL d'issue du texte. Il rend `None` — donc le chemin d'aujourd'hui —
dans trois cas, chacun un refus nommé :

1. **Aucune URL d'issue extractible.** Sans elle il n'y a pas de clé pour le terme d'état
   (§ D6), donc pas de moyen de savoir si le ticket porte une tâche de boucle, donc pas de
   droit d'écarter.
2. **Un `issue_comment` sur une PR.** Son URL est `…/pull/N#issuecomment-…` et le segment
   lu est `issues`, donc l'extraction échoue par construction. **C'est voulu** : les
   commentaires de PR restent hors (h). Les faire entrer demanderait un second terme
   d'état (`pr_has_active_task`) et surtout une seconde question — « un commentaire de PR
   porte-t-il une décision ? » — qui n'est pas celle du ticket. Le dire ici évite qu'un
   futur éditeur lise le trou comme un oubli.
3. **L'ordre des bras.** `Issue labeled ` est testé **avant** les nouveaux bras, comme
   dans `trigger_class` où l'ordre est déjà load-bearing. (c) `InertLabel` garde sa
   population ; (h) ne lui vole rien.

### D5 — Les classes, les releases, et pourquoi il y en a plusieurs

| ajout | rôle |
|---|---|
| `PrefilterClass::CommentTriagedNote` | le tour noté de la moitié `issue_comment` — AC5 |
| `Release::TriageEscalated` | le triage a vu une décision ⇒ tour complet — AC1 |
| `Release::TriageUnreadable` | fail-open : erreur, expiration, verdict illisible, provider absent — AC3 |
| `Release::IssueTaskActive` | le ticket **porte** une tâche de boucle ⇒ chemin d'aujourd'hui — AC7 |
| `Release::TriageDisarmed` | l'interrupteur dédié est à `off` (§ D8) |

Cinq noms et non un seul, pour une raison qui a déjà coûté : **deux populations sous un
même nom ne se soustraient pas** (mika#2205). `Release::StateUnreadable` existe déjà pour
les lectures d'état ; y verser les fail-open de triage rendrait impossible la question
« le triage est-il en train de tomber ? ». De même `SwitchDisarmed` nomme l'interrupteur
global : le dédié mérite le sien, sinon un désarmement local se lit comme un désarmement
global.

**Grammaire du verdict**, stricte et à une ligne :

```
DECISION
NOTE: <une ligne décrivant ce que le commentaire apporte>
```

Tout autre octet — préambule, JSON, deux lignes, un mot de plus — est **illisible**, donc
`Err`, donc fail-open. Le parse ne devine pas : une grammaire tolérante est une grammaire
dont le mode de panne est la note silencieuse, et c'est le danger central du ticket.

### D6 — Le terme d'état : une méthode de trait, un site SQL existant

`PrefilterState` gagne deux méthodes async, exactement comme (b)/(c)/(e) injectent leurs
lectures — c'est le seam qui garantit qu'**aucun test ne fait d'appel réseau** :

```rust
async fn issue_has_active_task(&self, issue_url: &str) -> anyhow::Result<bool>;
async fn triage(&self, text: &str) -> anyhow::Result<TriageVerdict>;
```

- `LiveState::issue_has_active_task` délègue à
  `AsyncDatabase::has_active_self_dev_task_for_issue` (`async_db.rs:962`, site SQL unique
  `db/tasks.rs:1739` par mika#2161 U3). `issue_url_variants` y couvre déjà les formes
  exacte et groom : pas de second parseur d'URL, pas de re-dérivation.
- **Normalisation.** `extract_issue_url` rend la ligne *trimée telle quelle*, fragment
  `#issuecomment-…` compris. La clé de tâche est l'URL **sans fragment** : la troncature
  est faite une fois, à l'extraction, avec son test d'unité. Sans elle, le terme d'état
  rendrait `false` sur tout commentaire et (h) écarterait des tickets en vol.
- `Err` sur l'un ou l'autre ⇒ `state_unreadable(what, target, &e)`, le fail-safe AC3 déjà
  en place.
- `LiveState` porte `llm: Option<&'a dyn LlmProvider>`. `None` ⇒ le triage est
  **indisponible** ⇒ `Release::TriageUnreadable` ⇒ tour complet. Un câblage oublié coûte
  l'économie, jamais la sûreté.

### D7 — La note : une troisième variante de `Decision`, écrite par la porte

```rust
Decision::SkipWithNote { class: PrefilterClass, target: String, note: String }
```

Traitée dans `skip_turn`, qui détient déjà `&AsyncDatabase`. Préférée à un
`note: Option<String>` ajouté à `Skip` : ce dernier aurait touché les six sites de
construction existants pour un champ `None` partout, et aurait rendu « écarté sans note »
et « écarté avec note » indiscernables au typage.

Écriture, dans cet ordre :

1. `db.add_event(description, Some(today), Some(notes))` — le chemin de `store_fact`
   catégorie `event` (`tools/store_fact.rs:302`), **sans passer par l'outil** : un outil
   est précisément ce que le tour léger n'a pas (AC6).
2. `db.index_content("event", Some(id), &content)` (`async_db.rs:2787`) — sans cet index
   la note n'est pas retrouvable par `search_memory`, et « consigne une note en mémoire »
   aurait livré une note invisible.
3. Violation d'unicité ⇒ la note existe déjà (webhook redélivré) : **bénin**, un INFO
   nommé, pas une erreur. Le dédoublonnage est offert par la contrainte, pas par un état.
4. Échec d'écriture ⇒ WARN nommé, et **le tour reste écarté**. La note est la trace, pas
   la décision — même forme et même raison que `webhook_prefilter_audit_failed` juste
   au-dessus.

### D8 — L'interrupteur dédié, et son défaut à `off` dans ce vol

`MIKA_WEBHOOK_COMMENT_TRIAGE`, lu par `comment_triage::triage_enabled()` avec la même
table de vérité que `prefilter_enabled()`. **Défaut : `off`.**

- `off` ⇒ `Decision::Llm(Release::TriageDisarmed)` : le chemin d'aujourd'hui à l'octet
  près, plus une ligne qui dit pourquoi.
- **Pourquoi désarmé** : le danger que le ticket déclare non négociable est « un ordre
  d'opérateur dégradé silencieusement en note », et le contrôle qui le mesure est AC1 —
  un corpus de commentaires réels passé au **vrai** modèle. Armer avant ce contrôle, c'est
  mettre en production précisément le risque. L'armement est donc le vol 2, et il est une
  ligne (§ Fire-Disposition).
- Un interrupteur dédié **en plus** du global, parce qu'un incident sur le triage ne doit
  pas obliger l'opérateur à désarmer les classes (a)–(e), qui sont déterministes,
  mesurées et sans brin de LLM.

---

## Périmètre — ce que CE vol livre

- La moitié **`issue_comment`** de (h) : candidat, classe, cinq releases, fenêtre de
  texte, terme d'état, décideur, note, interrupteur dédié.
- Le triage **réel** : prompt, appel borné, grammaire de verdict, parse strict, fail-open
  sur toutes ses branches.
- Le corpus **déterministe** d'AC2/AC3/AC5/AC6/AC7, la moitié mécanique d'AC1, et le
  contrôle de sensibilité (§ Verification Contract, V6).
- Comportement de production **inchangé** : l'interrupteur dédié est à `off`.

## Périmètre — ce que CE vol ne livre PAS

- **La moitié `issues closed`** (AC4). Même mécanisme, un bras de `classify` et une classe
  de plus ; elle attend le corpus réel pour être armée en même temps que sa jumelle.
- **Le corpus de commentaires réels sous vrai modèle** (AC1 au sens plein) et le rouge
  mesuré avec un triage qui note tout.
- **L'armement** : le passage du défaut de l'interrupteur à `on`.
- `MIKA_WEBHOOK_TRIAGE_MODEL` (§ D3), les commentaires de PR (§ D4), (g′) et les classes
  (a)–(g), inchangées.

**Tracked in:** senara-solutions/mika#2854 — vol 2 : la moitié `issues closed` (AC4), le
corpus réel sous vrai modèle (AC1), le rouge mesuré avec un triage qui note tout, et le
basculement du défaut de `MIKA_WEBHOOK_COMMENT_TRIAGE` à `on`. #2854 reste ouvert et
porte ce vol 2 jusqu'à sa livraison.

---

## Implémentation — ordre de commit

| # | portée | contenu |
|---|---|---|
| C1 | `server/upstream_close_handler.rs` | `extract_issue_url` passe `pub(crate)` (comme `extract_pr_url` l'a fait pour (e), pour la même raison : pas de quatrième copie du scanner) + troncature du fragment, avec son test d'unité |
| C2 | `server/comment_triage.rs` **(nouveau)** | `triage_enabled()`, `TRIAGE_SYSTEM_PROMPT`, `TriageVerdict`, `parse_verdict`, `run_triage` (appel borné, `tools: None`), `write_note` |
| C3 | `server/webhook_prefilter.rs` | `TriageHalf`, `Candidate::HumanTextNoTask`, bras de `classify`, `PrefilterClass::CommentTriagedNote`, les cinq releases, bras de `Touched::applies_to`, deux méthodes de `PrefilterState` + impls `LiveState`, `decide_comment_triage`, `Decision::SkipWithNote` + son bras dans `skip_turn`, amendement du doc de `PrefilterClass` (§ D1) |
| C4 | `server/handlers.rs` | `llm: Some(a.llm.as_ref())` dans le littéral `LiveState` |
| C5 | tests | le corpus déterministe du Verification Contract |

**Plancher du vol** — ce qui doit être vrai pour qu'il soit mergeable seul : C1→C4
compilent, `cargo test -p mika-agent` est vert, et la suite est verte **avec l'interrupteur
à `off`**, c'est-à-dire sur un comportement de production identique à `c5e64079`.

---

## Verification Contract

| # | ce qui est vérifié | comment | AC |
|---|---|---|---|
| V1 | un commentaire sans décision (accusé, note, lien) sur un ticket sans tâche ⇒ **une** note, **aucun** tour complet | `mika2854_run` e2e : `mock.calls_made() == 1` (le triage seul) et une ligne dans `skipped_by_class` | AC2, AC5 |
| V2 | un commentaire porteur d'une décision ⇒ tour complet | `mika2854_run` e2e : `calls_made() >= 2` (triage + tour), zéro ligne écartée, `release = triage_escalated` | AC1 (mécanique) |
| V3 | triage en `Err`, expiré, verdict illisible, provider absent ⇒ tour complet, **jamais** la note | quatre stubs de `PrefilterState::triage`, table-driven, sur `skip_turn` en direct ⇒ `Gate::Charged(TriageUnreadable)` | AC3 |
| V4 | la requête servie au triage ne porte **aucun** outil | assertion sur la `LlmRequest` construite : `request.tools.is_none()` — mesuré sur le tableau, pas sur la prose | AC6 |
| V5 | le verdict ne dépend pas de l'auteur | deux textes identiques à `by @vincent` / `by @un-tiers` près ⇒ même branche | D2 |
| V6 | **contrôle de sensibilité** : le corpus d'escalade rougit sous un triage qui note tout | le même corpus passé à un stub dégénéré « toujours `NOTE:` » ⇒ chaque cas atterrit sur `SkipWithNote` ; le test **assère cette bascule**, ce qui prouve que V2 n'est pas vert par construction | AC1 |
| V7 | un ticket **avec** tâche de boucle n'est pas touché | stub `issue_has_active_task -> Ok(true)` ⇒ `Charged(IssueTaskActive)`, et le triage **n'est pas appelé** (compteur sur le stub) | AC7 |
| V8 | interrupteur dédié à `off` ⇒ chemin d'aujourd'hui | `Charged(TriageDisarmed)`, aucun appel de triage, aucune note | D8 |
| V9 | la note est dédoublonnée et indexée | deux passes du même texte ⇒ un seul `event`, retrouvable par la recherche | D7 |
| V10 | le fragment est tronqué avant de servir de clé | test d'unité sur `extract_issue_url` : `…/issues/2854#issuecomment-123` ⇒ `…/issues/2854` | D6 |

**Contrainte de sérialisation, non négociable.** Tout test passant par
`mika2675_run`/`mika2675_run_on` — et donc par le nouveau `mika2854_run` — porte
`#[serial_test::serial]`. Deux états process-globaux le rendaient déjà nécessaire
(mika#2680) : `MIKA_WEBHOOK_PREFILTER` lu dans l'environnement, et l'`Interest` du callsite
`webhook_turn_charged` figé par le premier thread qui l'enregistre — un voisin non annoté
coupe la capture **globalement** (mesuré : quatre échecs sur la tête `6ba0762d` de la
PR #2705, job `Check`). Ce vol en ajoute un **troisième** :
`MIKA_WEBHOOK_COMMENT_TRIAGE`. Les tests qui exercent `skip_turn` en direct restent hors
de cette contrainte — ils reçoivent `enabled` en paramètre.

**Commandes :** `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check`, `scripts/verify-pipeline.sh origin/main`.

---

## Fire-Disposition

Ce plan livre des détecteurs — les dix lignes du Verification Contract sont des tests, et
leur chemin de succès est « aucune violation trouvée ». Disposition retenue :
**(b) livrer désarmé, avec son suivi tracké pour l'armer.**

**Ce qui atterrit désarmé, et par quel geste.** Le détecteur central du ticket — le corpus
de commentaires réels porteurs d'une décision, passé au **vrai** modèle (AC1 au sens
plein) — n'est pas dans ce vol, et le mécanisme qu'il surveille atterrit donc désarmé en
production : `MIKA_WEBHOOK_COMMENT_TRIAGE` a pour défaut `off` (§ D8). Le geste d'armement
est **une ligne** — le défaut passe à `on` — et il est délibérément couplé à la livraison
du corpus, dans le vol 2 tracké ci-dessus. La raison est celle du ticket lui-même : le
danger nommé non négociable est « un ordre d'opérateur dégradé silencieusement en note »,
et armer un discriminateur avant de l'avoir mesuré sur des ordres réels, c'est mettre ce
danger en production en espérant qu'il n'arrive pas.

**Ce qui atterrit armé, et sans exception.** V1 à V10 atterrissent **verts et armés**, sur
du code neuf : il n'existe aucune population préexistante de violations à excepter, donc
**aucune allowlist n'est livrée** — ni vide, ni peuplée. En créer une vide ici serait de
la cérémonie, et une allowlist sans population est exactement ce que mika#2309 condamne.

**Le contrôle de bonne foi.** Un corpus d'escalade peut être vert parce que le mécanisme
marche, ou vert parce qu'il n'assère rien. V6 sépare les deux : le même corpus repassé à
un stub dégénéré « toujours `NOTE:` » doit **basculer** sur `SkipWithNote`, et le test
assère la bascule. C'est la forme déterministe du « rouge mesuré avec un triage qui note
tout » que demande AC1 ; sa forme sous vrai modèle est au vol 2.

**Quand V6 rougit** : quelqu'un a rendu le routage insensible au verdict du triage — par
exemple en tolérant un verdict illisible comme une note. Restaurer le fail-open ; ne pas
excepter le cas.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/comment_triage.rs` (nouveau) | 130 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` (candidat, classe, releases, fenêtre, trait, décideur, `SkipWithNote`) | 125 |
| `crates/mika-agent/src/server/upstream_close_handler.rs` (visibilité + troncature du fragment + test d'unité) | 15 |
| `crates/mika-agent/src/server/handlers.rs` (câblage `llm` dans `LiveState`) | 8 |
| tests déterministes (`webhook_prefilter.rs`, `comment_triage.rs`, `server/mod.rs`) | 110 |

Total estimé : 388 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 388 < 429, avec 41 lignes de marge. La marge est mince et
c'est elle qui a dicté le découpage : le périmètre naturel du ticket — les deux moitiés de
(h), le corpus réel sous vrai modèle, et l'armement — pèse ≈ 560 lignes, soit 131 au-dessus
de la borne. Trois réductions ont été pesées et deux écartées :

- **écartée** : différer le corpus d'AC1 en gardant l'armement. Elle aurait livré en
  production un discriminateur dont la qualité n'est mesurée par rien, c'est-à-dire le
  danger central du ticket.
- **écartée** : différer la note de mémoire. C'est la moitié utile du tour léger ; sans
  elle le vol n'est observable par rien.
- **retenue** : différer la moitié `issues closed` (≈ 45 l) et le corpus réel sous vrai
  modèle (≈ 90 l), et livrer la moitié `issue_comment` désarmée. C'est la moitié qui porte
  la cellule chiffrée (13 tours, 1 007 748 tokens) et 79 % de la fenêtre mesurée ; sa
  jumelle réutilise le mécanisme sans le changer.

Noter que les doc-comments comptent dans l'estimation (seuls `docs/` et `CLAUDE.md` sont
exclus) et que ce module a une prose dense : les 125 lignes de `webhook_prefilter.rs`
incluent les docs des cinq releases et de la classe. Si la prose déborde pendant
l'implémentation, c'est la prose qui se resserre — pas un test qui tombe.

---

## Definition of Done

- [ ] C1→C5 commitées dans cet ordre, chacune compilant seule.
- [ ] `cargo test -p mika-agent` vert ; V1→V10 présents et verts.
- [ ] `cargo clippy --all-targets -- -D warnings` et `cargo fmt --check` propres.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] Tout test passant par `mika2854_run` porte `#[serial_test::serial]`.
- [ ] `MIKA_WEBHOOK_COMMENT_TRIAGE` documentée dans
      `docs/operator/environment-variables.md` : défaut `off`, table de vérité, ce que son
      armement change, et le fait que `MIKA_WEBHOOK_PREFILTER` la désarme en amont.
- [ ] Le doc-comment de `PrefilterClass` amendé (§ D1) : une classe peut désormais nommer
      un tour écarté **après** un brin de triage.
- [ ] Corps de PR : `Code review: delegated to mika-qa (mika#2694)`, la ligne
      `Tracked in:` du vol 2, et le défaut `off` dit explicitement comme un choix.
- [ ] Aucune classe (a)–(g) touchée ; aucun second écrivain de
      `webhook_prefilter_skipped`.

---

## Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#2854, § « Critères d'acceptation
(ébauche, à affiner au groom) ».

- **AC1 : corpus de contrôle négatif.** Un jeu de commentaires réels porteurs d'une
  décision : lever `operator-gated`, « je valide », fermer, lancer un vol, trancher
  Q1/Q2. On les tire des tickets de la file, par exemple les décisions du 2026-10-20 sur
  #2784, #2680, #2814 et #1381. **Chacun escalade vers le tour complet.** Le rouge est
  mesuré avec un triage qui note tout.
- **AC2 : contrôle positif.** Un jeu de commentaires sans décision (accusé, note, lien)
  donne une note, sans tour complet.
- **AC3 : fail-open.** Une erreur, un timeout ou une sortie illisible du triage donne le
  tour complet. C'est asserté.
- **AC4** : `issues` `closed` sans tâche suit la même règle que `issue_comment`.
- **AC5** : la ligne d'audit et `skipped_by_class` portent la classe (h) et la branche
  prise.
- **AC6** : le tour léger ne reçoit aucun outil mutationnel. C'est mesuré sur le tableau
  servi.
- **AC7** : un ticket **avec** tâche de boucle n'est pas touché, et passe par le chemin
  d'aujourd'hui.

**Couverture par vol.** Ce vol couvre AC2, AC3, AC5, AC6, AC7 en entier, et AC1 dans sa
moitié mécanique et déterministe (V2 + V6). **AC1 au sens plein** — le corpus réel passé
au vrai modèle, et son rouge mesuré — **et AC4** sont au vol 2, nommé par la ligne
`Tracked in:` ci-dessus. Le mécanisme livré par ce vol reste désarmé en production jusqu'à
ce que ces deux-là soient verts.

---

## Risques

| risque | parade |
|---|---|
| un ordre d'opérateur dégradé en note | double verrou : la grammaire stricte (tout verdict non conforme est un fail-open) **et** le défaut `off` jusqu'à ce que le corpus réel soit vert |
| la fenêtre de texte `by_any_handler` rend (h) inatteignable en production | vérifié : aucun handler ne réécrit un texte `New comment on ` ; `upstream_close_handler` se sélectionne sur `Issue closed:` et n'a que des effets de bord. La fenêtre stricte est le bon choix malgré tout : (h) lit **tout** le texte, et un texte réécrit est un texte que le pré-filtre n'a pas vu arriver |
| `extract_issue_url` sans troncature ⇒ (h) écarte un ticket en vol | V10, et la troncature est faite une seule fois, à l'extraction |
| un troisième état process-global casse la suite en parallèle | `#[serial_test::serial]` sur tout test e2e, dit au Verification Contract et dans la DoD |
| le triage devient un poste de coût | les deux branches écrivent leur release ; la prochaine mesure chiffre l'économie **et** le taux d'escalade sans nouvelle sonde |
| l'estimation déborde la borne | la prose se resserre avant qu'un test tombe ; si le code dépasse, la moitié `issue_comment` se livre sans la note et la note rejoint le vol 2 |
