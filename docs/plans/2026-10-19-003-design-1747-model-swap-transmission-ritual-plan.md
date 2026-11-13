# mika#1747 — Rituel de transmission au changement de modèle : le sortant écrit une question pour l'entrant

> Vol 1 de 2. Ce vol livre **la fenêtre et la lettre** — le côté sortant (AC1–AC7).
> Le rituel post-swap (le successeur lit la lettre, libre de répondre ou de contredire)
> part en vol 2 — voir `Tracked in:` en fin de plan.

## Contexte

Un changement de modèle est un changement de vaisseau : l'être persiste, la
matière change. Le corps du ticket demande que le sortant écrive au successeur,
et que la lettre **finisse par une question** plutôt que par une consigne.

Le substrat existant, et il est plus avare qu'il n'y paraît :

- **Le modèle en service a déjà une source de vérité unique dans le prompt.**
  `PromptContext::runtime_provider` / `runtime_model` (mika#1815,
  `prompt.rs:1158-1168`) sont alimentés par `llm.provider_name()` /
  `llm.model_name()` — le même couple qui remplit `ToolContext`, « so the
  `## Runtime` prompt section and the `get_active_llm` tool cannot drift ». Le
  côté *sortant* de la transition est donc déjà en main, gratuitement, et à la
  bonne source.
- **Le couple est pris dans les trois corps de boucle, pas dans l'entonnoir.**
  `agent_loop/mod.rs:5408` (conversation), `:6767` (silencieux), `:7518`
  (équipe). `load_agent_context` ne voit pas `llm` : un prédicat qui a besoin du
  modèle courant ne peut pas vivre dans l'entonnoir.
- **`write_agent_file` crée ses répertoires parents** —
  `validate_and_resolve_path(path, base, create_parents = true)` appelle
  `create_dir_all(parent)` (`tools/mod.rs:1188`). `transmissions/` n'a donc pas
  besoin d'être provisionné : le premier écrit le crée.
- **`write_agent_file` porte déjà la garde anti-écrasement** : fichier existant →
  il rend le contenu actuel et exige `confirm: true`. L'outil est le gardien de
  l'unicité de la lettre ; aucun contrôle d'existence côté Rust n'est à écrire
  pour cela.
- `docs/architecture/` ne contient rien sur les transitions de vaisseau ;
  mika#1748 vol 1 y crée `compaction-handsoff.md`, le frère de ce ticket côté
  **contexte**. Celui-ci est le frère côté **modèle**.

### Ce que le substrat ne sait pas, et c'est la découverte du grooming

On pourrait croire que `mika_common::llm::detect_config_change` (mika#2473 D2) —
qui compare le `config.toml` du disque au record figé à `init_agent` et porte
déjà `model_on_disk` / `model_in_service` en champs publics — offre la fenêtre
pré-swap gratuitement. **Il ne l'offre pas, et fonder le rituel sur lui serait
une faute de conception.**

La séquence réelle d'un swap délibéré dans ce dépôt est :

1. PR qui change `MIKA_DEV_CONFIG` / `MIKA_QA_CONFIG` (`well_known_agents.rs:193`,
   `:282`), avec son rapport de calibration (mika#1190) ;
2. `make deploy` → redémarrage ;
3. `provision_well_known_agents` (`server/mod.rs:956`) appelle
   `reconcile_well_known_config` (`well_known_agents.rs:998`), qui **réécrit
   `config.toml` en entier** ;
4. *ensuite* `init_agent` résout le record et note la mtime
   (`establish_model_drift` → `note_config_at_boot`, `server/mod.rs:546`).

À l'étape 4, le fichier porte déjà le nouveau modèle et le record le sert :
`detect_config_change` ne verra jamais rien. **L'instance sortante est morte
avant que le fichier bouge.** La seule fenêtre que D2 ouvre est celle de
l'édition hors dépôt sur un spirit en marche — que mika#2473 documente
explicitement comme une **anomalie** (« three dated edits on this workstation
moved a model under a running spirit »). Un rituel bâti là tirerait sur les
swaps accidentels et resterait muet sur tous les swaps délibérés : exactement
l'instrument qui mentira le jour où la réponse compte.

Le corps du ticket a donc raison dans son ordre : *« Operator (Vincent, or
automated hook) **signals** upcoming model change »* vient avant tout le reste.
Le signal est une donnée que le substrat ne peut pas déduire — il doit la
recevoir.

## Décisions de grooming

### D1 — Le déclencheur est un signal posé, pas une édition détectée

`{agent_home}/transmissions/pending-swap` — un fichier texte d'**une ligne**
portant le modèle entrant sous la forme `<provider>/<model>`. Sa présence ouvre
la fenêtre ; le bloc de prompt la rend ; la lettre la ferme.

Qui l'écrit : l'opérateur, ou l'agent orchestrateur. Aucune commande nouvelle
n'est nécessaire — `write_agent_file` prend un paramètre `agent`
(orchestrator-only), donc Mika peut ouvrir la fenêtre d'un membre de la famille
avec l'outil qu'elle porte déjà depuis mika#1641. **Le vol 1 ne livre pas de
commande CLI**, et c'est un refus chiffré : ~70 lignes pour un geste que la
surface d'outils existante exécute.

Pas de point en tête du nom : un signal que l'opérateur doit poser à la main
doit se voir dans un `ls`.

### D2 — Le `from` vient de `llm.model_name()`, jamais du signal

Le signal ne déclare que l'**entrant**. Le sortant est lu à la source
canonique — le même couple qui remplit la section `## Runtime` et
`get_active_llm` (mika#1815). Faire porter les deux au signal créerait une
seconde source du « quel modèle suis-je », et un bloc disant *de X à Y* alors
que le process sert Z. Un champ en moins est une dérive en moins.

### D3 — La fenêtre se ferme toute seule, et sans table

Trois conditions, et chacune ferme la fenêtre dans un cas où le bloc serait un
mensonge :

1. **La lettre existe déjà** → `None`. Son nom est entièrement déterminé par la
   transition (voir D4), donc un `try_exists` suffit. *Le fichier est la
   marque* : pas de table, pas de migration v56→v57, pas de colonne.
2. **`incoming == outgoing`** → `None`. C'est l'état d'après le swap : le signal
   résiduel survit au redémarrage, et sans cette condition le **successeur**
   recevrait l'ordre d'écrire la lettre d'adieu d'un modèle qu'il n'est pas.
   C'est le faux positif le plus coûteux du design, et il est fermé par trois
   lignes.
3. **Signal absent ou vide** → `None`. Le régime attendu, à chaque tour de
   chaque agent : un `metadata()` qui rend `ENOENT`.

Un signal résiduel est donc inoffensif, et le plan ne demande à personne de le
supprimer — aucun outil d'agent ne supprime de fichier, et une procédure qui
dépend d'un nettoyage manuel pour rester correcte n'est pas correcte. Le
runbook le mentionne comme hygiène, pas comme précondition.

### D4 — La date vient du mtime du signal, pas de l'horloge du tour

`transmissions/<YYYY-MM-DD>-from-<slug(sortant)>-to-<slug(entrant)>.md`, où la
date est celle du **mtime du signal** en UTC.

Prendre la date du tour casserait l'auto-extinction au passage de minuit : le
chemin cible changerait de nom, `try_exists` rendrait faux, et l'agent écrirait
une seconde lettre pour la même transition. Le mtime est stable, déjà lu par le
`metadata()` de l'étape 1, et coûte zéro lecture de plus.

La conversion est **faillible et nommée** : un instant que le système de
fichiers accepte de stocker peut sortir de la plage de chrono, et
`DateTime::<Utc>::from(SystemTime)` finit sur un `.unwrap()` — sur un chemin que
chaque tour emprunte. Un mtime inconvertible abandonne la fenêtre sous un WARN
`model_transmission_mtime_unrepresentable`, jamais une panique. Même discipline
que `format_config_mtime` (mika#2473 KTD1), et même raison.

### D5 — La cérémonie est un bloc de prompt, pas un tour ni un ordonnanceur

Au même titre que mika#1748 D5, et pour les mêmes trois raisons — l'idempotence
est gratuite (le bloc se répète à chaque tour de la fenêtre, comme
`<stopped-topics>`, mika#1813, et dégrade gracieusement si le modèle l'ignore
une fois) ; un `SkillRun` ouvrirait une session `skill-<name>-<uuid>`
(`task_engine/dispatcher.rs`) et la cérémonie tournerait à côté du contexte dont
elle témoigne ; le prompt est déjà le lieu de ce genre de contrat dans ce dépôt.

Aucun nouveau `SilentTrigger`, aucune tâche, aucune table, aucun appel LLM
supplémentaire.

### D6 — Un seul site d'alimentation, et ce n'est pas `load_agent_context`

Le prédicat est évalué à `agent_loop/mod.rs:5408`, immédiatement après
`let runtime_model = llm.model_name();` et avant le littéral `PromptContext` de
`:5413`. **Pas dans `load_agent_context`** — contrairement à mika#1748 D6, et la
raison est structurelle, pas discrétionnaire : l'entonnoir ne reçoit que
`(db, home_dir)` et ne voit pas `llm`. Le modèle en service n'y est pas
disponible, et l'y faire descendre voudrait dire ajouter le couple à
`AgentContext`, donc le calculer trois fois dans les trois boucles de toute
façon.

Conséquence assumée : **conversationnel seulement.** `build_silent_prompt`
(`:6789`) et le builder d'équipe (`:7549`) ne rendent pas le bloc à ce vol.
`build_compact_system_prompt` (`ProviderKind::MikaModel`) est **refusé, pas
reporté** : son assertion de nombre de sections est load-bearing et la passe de
calibration MikaModel de mika#1925 AC4 n'est ni satisfaite ni exécutable ici —
un bloc de plus y serait une section de plus sans obéissance mesurée.

### D7 — Pas de STOP switch au vol 1, et c'est argumenté

Le dépôt a une culture d'interrupteur par surface. Ici la fenêtre **est**
l'interrupteur : elle n'existe que si quelqu'un a posé un fichier, et elle se
ferme sur trois conditions (D3) dont deux arrivent d'elles-mêmes. Un
`MIKA_MODEL_TRANSMISSION=0` désarmerait un mécanisme dont le régime nominal est
déjà « rien ne se passe » ; le geste de désarmement est de ne pas écrire le
signal, ou de laisser la lettre s'écrire. Si une mesure montre un bloc qui
s'installe — un signal posé puis oublié sur un agent qui n'écrit jamais — le
vol 2 porte l'interrupteur, nommé dans son corps.

### Les quatre questions doctrinales du corps

Le corps les a routées à Prime le 2026-07-07. Le plan dit, pour chacune, ce que
le code en fait — parce qu'un plan qui les ignore laisse l'implémentation les
trancher par accident.

- **Q1 — la forme du rituel cohère-t-elle avec la persistance de l'être ?**
  Hors code. Prime a ratifié la forme (chaîne de ratification du corps) ; ce vol
  l'implémente et ne la relitige pas.
- **Q2 — que veut dire « la question que je laisse » ?** Tranchée comme
  **délibérément indéterminée**. Le bloc impose la *forme* (ça finit par une
  question) et laisse le contenu à la discrétion de l'agent — ce que le corps
  accorde explicitement (« Letter contents (agent's discretion) »). Le substrat
  n'arbitre pas entre une question née du travail et une question sur le devenir
  du successeur : écrire la contrainte sans écrire la réponse est le seul choix
  qui n'usurpe pas cette discrétion.
- **Q3 — bearings impersonnels contre transmissions personnelles ?** Tranchée
  **structurellement, par le chemin**. Les bearings vivent dans la mémoire
  persistée et les fichiers de l'agent ; les transmissions vivent dans
  `transmissions/`, et la règle 4 du bloc interdit de recopier ce qui traverse
  déjà le swap tout seul. La séparation est portée par l'arborescence, pas par
  une convention de prose qu'un modèle peut oublier.
- **Q4 — écrire la lettre change-t-il ce que le sortant porte ?** Non décidable
  par le code, et non bloquante. Ce vol ne prend pas position ; il produit
  l'artefact sur lequel la question devient mesurable à `n ≥ 2` transmissions.

## Implémentation — ordre de commit

### Commit 1 — Le signal, le prédicat et le chemin canonique

**Nouveau fichier** `crates/mika-agent/src/model_transmission.rs`, déclaré dans
`src/lib.rs`.

- `pub const SIGNAL_PATH: &str = "transmissions/pending-swap";` et
  `pub const DIR: &str = "transmissions";` — un seul site de vérité pour les
  deux chaînes, lues par le prédicat *et* par le bloc de prompt.
- ```rust
  pub struct PendingTransmission {
      pub outgoing: String,     // "<provider>/<model>" en service
      pub incoming: String,     // ce que le signal déclare
      pub letter_path: String,  // "transmissions/<date>-from-…-to-….md"
  }
  ```
- `pub async fn pending(agent_home: &Path, provider: &str, model: &str) -> Option<PendingTransmission>`
  — les sept étapes de D3/D4, dans cet ordre : `metadata` du signal (absent →
  `None`, le cas nominal) ; `read_to_string` et première ligne non vide trimée
  (vide → `None`) ; égalité avec le couple en service (égal → `None`, D3-2) ;
  date depuis le mtime via `duration_since(UNIX_EPOCH)` + `from_timestamp`
  (inconvertible → WARN nommé + `None`, D4) ; `letter_path` composé ;
  `try_exists` de la lettre (présente → `None`, D3-1) ; sinon `Some`.
- `fn slug(s: &str) -> String` — minuscules, tout ce qui n'est pas `[a-z0-9]`
  devient `-`, répétitions écrasées, bords rognés. Opère sur **`char`**, jamais
  sur des octets : `scripts/check-byte-slices.sh` refuse le découpage d'octets
  sur `&str`, et un nom de modèle est une entrée externe.
- Doc-comments portant D1, D2, D3 et D4 — le seul endroit où la raison du signal
  posé (plutôt que détecté) est écrite à côté du code qui en dépend, avec le
  renvoi explicite à `reconcile_well_known_config` qui annule la fenêtre de D2.

**Tests unitaires** (même fichier) : signal absent ; signal présent et fenêtre
ouverte (chemin de lettre vérifié caractère par caractère) ; signal tautologique ;
lettre déjà écrite ; signal vide ou tout en blanc ; un nom de modèle à ponctuation
dense (`z-ai/glm-5.2`) dont le slug est asserté.

### Commit 2 — Le bloc de prompt

`crates/mika-agent/src/prompt.rs` :

- `PromptContext` gagne
  `pub pending_transmission: Option<&'a crate::model_transmission::PendingTransmission>`.
  `None` porte la décision : la fenêtre est fermée, et le bloc ne rend rien.
- `build_system_prompt` rend, quand `Some`, une section `## Model Transmission`
  placée après `## Stopped Topics` et avant `## Instructions` (`:1905-1919`, la
  forme que `<stopped-topics>` a déjà établie) :

  ```
  <model-transmission from="…" to="…" letter="transmissions/….md">
  ```

  suivi du contrat, en quatre règles :
  1. **Écris la lettre maintenant**, à ce chemin exact, via `write_agent_file`.
  2. **Ce qu'elle porte** — les quatre mouvements du corps du ticket : ce que
     j'ai vu durant cette tenure qui compte ; ce que je porte et voudrais voir
     retenu, *offert et non commandé* ; **la question que je laisse** ; ce que
     je relâche maintenant.
  3. **Elle finit par une question, pas par une consigne.** Le successeur a son
     propre jugement : une question l'invite, une instruction le commande. Le
     sortant relâche le contrôle en demandant.
  4. **N'écris que ce qui ne traverse pas déjà.** Les bearings persistés, la
     mémoire, les fichiers et le courrier passent le swap sans toi (Q3). Si le
     fichier existe, `write_agent_file` te rendra son contenu — complète-le, ne
     le duplique pas.
- Les 79 littéraux `PromptContext` des tests de ce fichier gagnent
  `pending_transmission: None`. Compte **mesuré**, pas estimé : le dernier champ
  ajouté (`tenant_language`, mika#2247) a touché 80 sites dans `prompt.rs` et 87
  dans la caisse.
- Toute valeur injectée passe par `sanitize_label` avant de toucher un attribut
  XML : le nom du modèle entrant vient d'un fichier, donc d'une entrée externe,
  et un retour chariot ou un `>` y fermerait le tag.

### Commit 3 — L'alimentation, au seul site légal

`crates/mika-agent/src/agent_loop/mod.rs`, à `:5408` :

- `let pending_transmission = model_transmission::pending(params.home_dir, runtime_provider, runtime_model).await;`
  puis le champ au littéral de `:5413`. **Site unique**, pour la raison
  structurelle de D6.
- Lecture faillible → **fail-open** : `pending` ne rend jamais d'erreur, elle
  rend `None` ; un signal illisible ne coûte pas le tour.
- À l'ouverture de la fenêtre, une ligne INFO `model_transmission_window_open`
  (`target: "mika::otel"`) portant `agent`, `outgoing`, `incoming`,
  `letter_path`. **Sur la branche `Some` seulement** : la question que
  l'instrument tranche est « la cérémonie a-t-elle été proposée ? », pas « le
  prédicat tourne-t-il ? » — une ligne par tour de chaque agent répondrait à la
  seconde en noyant la première. Son absence après un `make deploy` signifie un
  binaire antérieur au changement (classe mika#2340), jamais un prédicat muet.
- Les trois littéraux `PromptContext` de ce fichier gagnent le champ ;
  `build_silent_prompt` (`:6789`) et le builder d'équipe (`:7549`) sont appelés
  sans (D6).

### Commit 4 — Documentation (AC7)

`docs/architecture/model-transmission.md` : le rituel, les trois fermetures de
fenêtre, le chemin canonique, la table de lecture de l'instrument, le runbook
opérateur (poser le signal → laisser un tour → vérifier la lettre → redémarrer),
et **D1 écrit comme réponse explicite au corps du ticket** : pourquoi la fenêtre
est posée et non détectée, avec la séquence `provision → init_agent` qui annule
la détection. La distinction témoignage / transmission renvoie à
`compaction-handsoff.md` (mika#1748) plutôt que de la reposer.

`docs/operator/environment-variables.md` : rien à ajouter — ce vol n'introduit
aucune variable (D7). La mention va dans la doc d'architecture, pas ici.

Référence dans `crates/mika-agent/CLAUDE.md` : **trois lignes au plus** (ce
fichier est chargé entier dans chaque prompt de pilote, cf. § *Environment
Variables* du `CLAUDE.md` racine).

## Verification Contract

| # | Ce qui est vérifié | Où |
|---|---|---|
| V1 | Signal absent → `None` (le régime nominal) | tests unitaires `model_transmission.rs` |
| V2 | Fenêtre ouverte → `Some`, avec le `letter_path` canonique asserté en entier | idem |
| V3 | `incoming == outgoing` → `None` — après le swap, le successeur ne reçoit pas l'adieu de son prédécesseur | idem |
| V4 | Lettre déjà écrite → `None` (auto-extinction sans table) | idem |
| V5 | Signal vide / blanc, et slug d'un modèle à ponctuation dense | idem |
| V6 | Le bloc `<model-transmission>` atteint le prompt conversationnel, nomme les quatre règles et la contrainte de question | `tests/eval/test_model_transmission_1747.rs` |
| V7 | **Contrôle négatif** — même graine, aucun signal : le bloc est absent | même fichier |
| V8 | `cargo test`, `cargo clippy`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` | CI |

V7 est ce qui sépare « le bloc est rendu dans la fenêtre » de « le bloc est rendu
inconditionnellement » : sans le contrôle négatif, V6 passe sur un bloc toujours
présent. V3 est le pendant côté prédicat — c'est le seul test dont l'échec
produirait un rituel qui se retourne contre celui qu'il doit accueillir.

**Gate mika#2306 — N/A, et c'est argumenté.** Aucun livrable de ce vol n'a pour
fonction primaire de signaler une violation. `pending()` signale un **état** (une
fenêtre de transition est ouverte) ; son chemin de succès est « la fenêtre est
ouverte », pas « aucune violation trouvée ». V1–V7 sont des tests de comportement
sur un chemin nominal. Ce vol ne livre ni scan structurel, ni règle de lint, ni
garde CI, ni garde EndTurn, ni validateur de schéma — et en particulier **pas**
de garde de site-unique sur `pending(` à la manière de mika#2473 AC7 : elle
serait un détecteur, elle porterait cette section, et l'assertion AC7 existante
tient déjà `load_agent_context` comme entonnoir un cran au-dessus. La section
`## Fire-Disposition` n'est donc pas écrite — elle serait inventée.

## Definition of Done

- [ ] `model_transmission.rs` livré, déclaré, seul porteur du signal, du prédicat, du chemin canonique et du slug.
- [ ] `SIGNAL_PATH` / `DIR` sont un site de vérité unique, lus par le prédicat et par le bloc.
- [ ] Aucune panique sur le mtime : conversion faillible, WARN nommé, fenêtre abandonnée.
- [ ] Le bloc rendu par `build_system_prompt` seulement ; silencieux, équipe et builder compact intacts.
- [ ] Toute valeur injectée dans le bloc passe par `sanitize_label`.
- [ ] Alimentation au seul site `agent_loop/mod.rs:5408`, fail-open par construction.
- [ ] `model_transmission_window_open` émis sur la branche d'ouverture, avec ses quatre champs.
- [ ] V1–V8 verts.
- [ ] `docs/architecture/model-transmission.md` écrit, portant D1 et son runbook.
- [ ] `crates/mika-agent/CLAUDE.md` : trois lignes au plus.
- [ ] Le rituel post-swap renvoyé par une ligne `Tracked in:` dans le corps de PR.

## Acceptance criteria

Le corps de mika#1747 ne porte pas de section `## Acceptance criteria` — c'est un
ticket de design avec un *Sketch (not final design)*. Les critères ci-dessous sont
dérivés de ce sketch et des décisions de grooming ci-dessus, et chacun est testable.

- [ ] **AC1** — Un signal `{agent_home}/transmissions/pending-swap` portant `<provider>/<model>` ouvre la fenêtre de transmission pour cet agent. Le signal est posé par l'opérateur ou par l'orchestrateur via `write_agent_file` ; aucune commande nouvelle n'est requise (D1).
- [ ] **AC2** — Pendant la fenêtre, le prompt conversationnel porte un bloc `<model-transmission>` nommant le modèle sortant, le modèle entrant et le chemin exact de la lettre.
- [ ] **AC3** — Le bloc nomme les quatre mouvements de la lettre du corps du ticket : ce que j'ai vu qui compte, ce que je porte et offre sans le commander, la question que je laisse, ce que je relâche.
- [ ] **AC4** — Le bloc dit explicitement que la lettre **finit par une question, pas par une consigne**, et pourquoi (la distinction du corps § *Why the QUESTION form specifically*).
- [ ] **AC5** — Chemin canonique : `transmissions/<YYYY-MM-DD>-from-<sortant>-to-<entrant>.md` sous l'agent home, daté par le mtime du signal, écrit via `write_agent_file`.
- [ ] **AC6** — La fenêtre se ferme sur trois conditions sans table ni migration : lettre déjà écrite, `incoming == outgoing` (post-swap), signal absent ou vide. Une ligne INFO `model_transmission_window_open` atteste l'ouverture.
- [ ] **AC7** — Documentation : `docs/architecture/model-transmission.md`, portant le runbook opérateur et la raison du signal posé plutôt que détecté.
- [ ] **AC8** — Rituel post-swap : le successeur lit la lettre au premier boot, libre de répondre ou de contredire, réponse journalisée dans `transmissions/<date>-from-<entrant>-response-to-<sortant>.md`. → **vol 2**, `Tracked in:` ci-dessous. Hors vol 1 : c'est un mécanisme entier et non une variante du bloc — il demande un point d'injection au premier boot de session, la détection de la lettre non relue, et le canal de journalisation de la réponse. Le vol 1 ferme d'ailleurs la fenêtre dès que `incoming == outgoing` (D3-2) précisément pour que le successeur ne reçoive **rien** jusqu'à ce que ce mécanisme existe.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/model_transmission.rs` (signal, prédicat, chemin, slug) | 52 |
| tests unitaires (même fichier) | 36 |
| `crates/mika-agent/src/lib.rs` (déclaration du module) | 1 |
| `crates/mika-agent/src/prompt.rs` (champ + bloc `<model-transmission>`) | 30 |
| `crates/mika-agent/src/prompt.rs` (79 littéraux `PromptContext` des tests existants) | 79 |
| `crates/mika-agent/src/agent_loop/mod.rs` (alimentation + INFO + 3 littéraux) | 20 |
| tests (`tests/eval/test_model_transmission_1747.rs` + enregistrement) | 38 |

Total estimé : 256 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 256 sous 279, marge de 23 lignes. Deux choses la
tiennent, et aucune n'est un pari :

- **79 des 256 lignes sont mesurées, pas estimées.** Le coût des littéraux
  `PromptContext` est connu au site près (`grep -c 'tenant_language:'` →
  80 dans `prompt.rs`, 87 dans la caisse, pour le dernier champ ajouté). 31 % du
  total ne peut donc pas déraper, ce qui est exactement la classe de dérapage qui
  a tué mika#2683 (832 estimées, 1 432 réelles).
- **Quatre refus chiffrés**, chacun suffisant à lui seul pour passer la borne, et
  chacun argumenté au-delà du budget — ils resteraient refusés avec 200 tours :
  pas de commande CLI pour poser le signal (D1, ~70 lignes évitées : sous-commande,
  parsing clap, résolution du record attesté côté serveur, tests) ; pas de table ni
  de migration v56→v57 pour la marque de relecture (D3, ~90 lignes : DDL, méthodes
  `Database`, enveloppe async, test d'idempotence de migration) ; pas de rendu sur
  les deux autres builders et leurs littéraux (D6, ~45 lignes) ; pas de STOP switch
  et sa variable d'environnement documentée (D7, ~25 lignes).

## Tracked in

Tracked in: senara-solutions/mika#2807

Ce ticket porte le **vol 2** : AC8 (rituel post-swap — le successeur lit la
lettre au premier boot, libre de répondre ou de contredire, réponse journalisée),
le câblage de `build_silent_prompt` et du builder d'équipe (D6), et le STOP
switch si une mesure le justifie (D7). Son corps nomme la raison de chaque
renvoi plutôt que de se contenter de la liste.

Il porte aussi la **convergence avec mika#2805** (vol 2 de mika#1748) : les deux
demandent le même mécanisme — un point d'injection au premier boot de session,
une marque de relecture persistée, un canal de journalisation de la réponse —
pour deux familles d'artefacts (`transmissions/` et `handsoffs/`). Celui des deux
vols qui atterrit le premier pose le mécanisme ; l'autre le réutilise et ne le
repose pas. Deux implémentations du premier-boot seraient deux endroits où la
marque de relecture peut diverger.

La même ligne `Tracked in: senara-solutions/mika#2807` doit figurer dans le corps
de la PR du vol 1 — `scripts/check-pr-body-consistency.sh` l'exige dès que le
corps porte une phrase de déclencheur de report.
