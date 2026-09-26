---
title: "La porte ready doit avoir une liste — allowlist de depots dispatchables sur les deux couches de garde"
date: 2026-08-29
issue: senara-solutions/mika#2046
branch: bug/2046/loop-substrate-rien-n-emp-che-la-boucle
type: fix
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
depth: standard
risk: high
---

# La porte ready doit avoir une liste

## Goal Capsule

- **Objectif (resultat).** Qu'un depot que la boucle n'a pas le droit de toucher ne puisse pas etre touche par elle, quel que soit le clic qui le demande. Aujourd'hui un operateur qui trie un backlog et pose `ready` sur une issue de `control-monitor` lance un pipeline autonome dans un depot dont la doctrine dit qu'il est spawn-CC-uniquement — et rien, nulle part, ne s'y oppose. Le resultat vise se verifie de l'exterieur du moteur : poser `ready` sur un depot hors liste laisse une trace nommee et aucun worktree.
- **Moyen (approche retenue).** Une constante d'allowlist et son predicat dans `crates/mika-agent/src/webhook_dispatch.rs` — le module qui se declare deja source unique pour les deux couches de garde — consommes par le handler pre-LLM `ready_label_handler` et par la garde de frontiere d'outil `validate_dispatch_readiness` (KTD1, KTD2, KTD3).
- **Autorite.** Le corps de mika#2046 fixe l'intention. La liste exacte (`mika`, `mika-cloud`, `mika-skills`, `mika-platform`) est un arbitrage operateur rendu le 2026-08-29, que le ticket laissait explicitement ouvert. Sur la forme du refus et le nombre de couches, `docs/solutions/architecture-patterns/post-hoc-vs-tool-boundary-guard-placement-2026-05-13.md` prime sur la lettre du ticket : le ticket ne nomme que `ready_label_handler`, le learning du depot etablit que cette couche seule n'est pas porteuse.
- **Conditions d'arret.** S'arreter et surfacer si l'implementation constate que le refus en `Handled` ne desarme pas l'INTENT_GUARD `webhook_ready_label_dispatch` — cela invaliderait KTD1 et le correctif serait pire que rien, puisque le guard forcerait le dispatch que la garde vient de refuser.

---

## Product Contract

### Resume

Le label `ready` est une porte ouverte. Les quatre maillons qui menent du webhook GitHub au worktree — le parse du marqueur, la resolution du nom de depot, le parse du prompt de `dispatch-lib`, la resolution du repertoire de travail — font tous de la manipulation de chaine et aucun ne demande si le depot nomme a le droit d'etre dispatche. La valeur par defaut est « tout ce que le webhook nomme ». Elle doit devenir « rien, sauf ce qui est liste ». Le correctif pose cette liste a un seul endroit, la fait consommer par les deux couches de garde que le depot reconnait comme les siennes, et fait citer la liste par le refus lui-meme.

### Problem Frame

La decision operateur du 2026-08-29 11:15 etablit que `control-monitor` et `claude-pilot` sont spawn-CC-uniquement, jamais la boucle mika. `control-monitor#98` et `claude-pilot#79` ont ete fermes en won't-do par cette meme decision. La mise en oeuvre en cours est une couche de convention : des pointeurs de commandes `/control-monitor` et `/claude-pilot`. Une commande guide celui qui la lit ; elle n'arrete pas celui qui clique.

Verification faite sur ce worktree a `c936003c` — les quatre maillons, aucun filtre :

| maillon | ce qu'il fait | filtre ? |
|---|---|---|
| `crates/mika-agent/src/server/ready_label_handler.rs` — `ReadyLabelLocation::owner_repo()` | `format!("senara-solutions/{}", self.repo_ref)`, `repo_ref` venant du payload webhook | non |
| meme fichier — `ReadyLabelLocation::repo_name()` | `rsplit_once('/')`, pure manipulation de chaine | non |
| `skills/bundled/_shared/dispatch-lib.sh:1077` | `^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$` — le tiret est autorise | non — `control-monitor#159` et `claude-pilot#119` passent |
| `skills/bundled/_shared/dispatch-lib.sh:1088-1093` | `SUB_REPO_DIR="$PLATFORM_DIR/$REPO"` puis test de presence `.git` | non — les deux repertoires sont des depots git presents dans le workspace |

**Corrections d'evidence portees depuis la verification de code.** Le corps du ticket cite la regex `^[a-zA-Z0-9_-]+#[0-9]+$` ; la forme reellement en place rend le prefixe owner optionnel (`dispatch-lib.sh:1077`). Le ticket situe les occurrences `senara-solutions/...` en commentaire a `:898` et `:955` ; elles sont a `:1069-1070`. Ces deux derives ne touchent pas le fait porte par le ticket : le tiret passe, et rien ne valide.

Ce que le ticket ne dit pas et que la lecture du moteur revele : **le refus le plus evident est un piege**. `ready_label_dispatch_trigger` (`crates/mika-agent/src/agent_loop/mod.rs:6436`) declenche sur `msg.starts_with(READY_LABEL_DISPATCH_MARKER)`. Un refus qui rendrait `VerdictAction::Passthrough` laisse le texte du marqueur intact, donc l'INTENT_GUARD `webhook_ready_label_dispatch` (`mod.rs:6304`) se declenche apres le tour LLM et harcele le modele — son `correction_message` dit « The turn continues until the appropriate dispatch tool is called ». La garde moteur serait annulee par la couche suivante.

### Requirements

- R1. Une liste explicite de depots dispatchables existe a un seul endroit du code, et la valeur par defaut pour un depot absent de la liste est le refus.
- R2. La liste contient exactement `mika`, `mika-cloud`, `mika-skills`, `mika-platform` sous l'owner `senara-solutions` (arbitrage operateur du 2026-08-29).
- R3. Le handler pre-LLM `ready_label_handler` refuse un depot hors liste **avant** toute creation de tache, et avant le `gh issue view` que l'etape 4 declencherait.
- R4. Ce refus ne laisse pas l'INTENT_GUARD `webhook_ready_label_dispatch` reclamer un dispatch : il remplace le texte du marqueur au lieu de le laisser passer (voir KTD1).
- R5. La frontiere d'outil `validate_dispatch_readiness` refuse un `run_claude_pilot` / `run_claude_pilot_groom` dont le `prompt` nomme un depot hors liste, quelle que soit l'origine du tour — c'est la couche porteuse au sens de `post-hoc-vs-tool-boundary-guard-placement-2026-05-13.md`.
- R6. Chaque refus est bruyant et nomme : evenement `ready_label_repo_not_dispatchable` pour la couche webhook, `repo_not_dispatchable` pour la frontiere d'outil, avec depot et numero en champs structures exploitables — jamais un silence.
- R7. Le refus cite la liste qui l'a motive, de sorte que le lecteur du log sache immediatement ce qui aurait ete accepte.
- R8. La validation porte sur la paire owner+depot, pas sur le basename seul : un marqueur `autre-org/mika#1` est refuse.
- R9. Les tests couvrent les deux sens sur chaque couche : `mika#N` et `mika-cloud#N` continuent de dispatcher ; `control-monitor#N` et `claude-pilot#N` sont refuses avec l'evenement nomme.

### Key Decisions

- **La liste est une constante Rust, pas une derivation depuis l'etat du workspace.** Gouverne R1, R2. Deriver la liste de « quels depots git sont presents » est precisement le predicat qui echoue : `control-monitor` et `claude-pilot` *sont* des depots git presents. Le ticket ouvre cette porte (« derivee ... ou, a defaut, tenue a un seul endroit que le refus cite ») ; la seconde branche est la seule correcte ici. Precedent maison : `docs/solutions/1053-dispatch-trigger-allowlist-config-constant.md`.
- **La correction porte deux couches, pas une.** Gouverne R3, R5. Le ticket ne nomme que `ready_label_handler` ; son titre dit « la chaine entiere ». Le learning du depot classe `run_claude_pilot` en outil a effet de bord etatique, pour lequel « tool-boundary check required ». Voir KTD2.
- **La couche shell reste hors perimetre.** Gouverne les frontieres de scope. Voir KTD5.

### Scope Boundaries

Dans le perimetre : la constante et son predicat dans `webhook_dispatch.rs` ; la garde pre-LLM dans `ready_label_handler.rs` ; la garde de frontiere d'outil dans `skills/executor.rs` ; les tests des deux sens sur les trois sites.

Hors perimetre :

- **Les pointeurs de commandes** (`/cm` vers `/control-monitor`, `/mika` vers `/claude-pilot`). Couche convention, traitee ailleurs, et elle reste utile au-dessus de la garde.
- **Une garde dans `skills/bundled/_shared/dispatch-lib.sh`.** Troisieme couche, autre langage, et le chemin y arrive deja filtre par la garde de frontiere d'outil. Ticket separe a deposer (KTD5) — discipline `feedback_implementation_scope_bundling`.
- **Le choix des depots de la liste.** Arbitrage rendu ; ce plan l'applique, il ne le rouvre pas.

### Sources

- `crates/mika-agent/src/webhook_dispatch.rs:1-5` — « Both `agent.rs` (INTENT_GUARDS post-hoc) and `skills/executor.rs` (tool-boundary pre-hoc) consume these predicates. Single source of truth prevents drift between the two guard layers. » C'est la justification du site de la constante.
- `crates/mika-agent/src/agent_loop/mod.rs:6436` et `:6304` — le trigger de l'INTENT_GUARD et son `correction_message`. Origine de KTD1.
- `crates/mika-agent/src/server/ready_label_handler.rs:1008-1009` — les asserts qui etablissent que les deux pre-digests commencent par `<ready_label_handler>` ; `:550` documente que c'est intentionnel pour ne pas matcher le trigger du guard.
- `crates/mika-agent/src/skills/executor.rs:1037-1057` — le rejet `unauthorized_webhook_dispatch` (mika#933), modele exact du rejet a ajouter.
- `docs/solutions/architecture-patterns/post-hoc-vs-tool-boundary-guard-placement-2026-05-13.md` — la table de reversibilite qui classe `run_claude_pilot` et impose la frontiere d'outil.
- `docs/solutions/1053-dispatch-trigger-allowlist-config-constant.md` — le precedent « allowlist en constante Rust » et ses quatre raisons.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Le refus rend `VerdictAction::Handled`, jamais `Passthrough`.** Gouverne R4. `Passthrough` laisse `req.text` egal au marqueur brut (`crates/mika-agent/src/server/handlers.rs:1280-1286`), donc `ready_label_dispatch_trigger` se declenche et l'INTENT_GUARD exige un dispatch que la garde vient de refuser. `Handled` remplace `req.text` par le pre-digest ; en le faisant commencer par `<ready_label_handler>` comme les deux pre-digests existants, le guard ne matche plus — c'est le mecanisme deja documente en `ready_label_handler.rs:550` (« composes by construction with no flag threading »). Le pre-digest dit au modele que le depot n'est pas dispatchable, cite la liste, et lui demande de prevenir l'operateur sans dispatcher.
- KTD2. **Deux couches, avec des roles distincts.** Gouverne R3, R5. La garde pre-LLM empeche la pre-creation de tache et le dispatch moteur pour le chemin webhook ; la garde de frontiere d'outil empeche l'execution de `run_claude_pilot` quelle que soit l'origine du tour. Le learning maison est explicite : pour un outil a effet de bord etatique, le guard post-hoc « is not load-bearing for the side-effect prevention ». Sans la seconde couche, un tour non-webhook (demande en langage naturel, chemin de recuperation, skill qa/ci) atteint encore `control-monitor`. Les deux coexistent exactement comme mika#933 et mika#910 coexistent.
- KTD3. **La constante et le predicat vivent dans `crates/mika-agent/src/webhook_dispatch.rs`.** Gouverne R1, R7. Le module se declare deja source unique pour les deux consommateurs vises, et c'est deja de la il que `is_unauthorized_webhook_dispatch` est consommee par `executor.rs`. Y ajouter la liste evite la derive entre couches que le module existe pour empecher. Le predicat expose aussi la liste sous forme affichable pour que chaque refus la cite (R7).
- KTD4. **La validation porte sur owner+depot.** Gouverne R8. `repo_name()` seul laisserait passer `autre-org/mika#1`, car son basename est `mika`. Le predicat prend la forme pleinement qualifiee produite par `owner_repo()` — qui applique deja l'owner par defaut `senara-solutions` pour les marqueurs courts — et la compare a la liste qualifiee. Un marqueur court `mika#1` devient `senara-solutions/mika` et est accepte ; `autre-org/mika#1` ne l'est pas.
- KTD5. **La garde shell de `dispatch-lib.sh` part en ticket separe.** Gouverne les frontieres de scope. Le fait est reel et constate dans ce plan, mais c'est une troisieme couche dans un autre langage, atteinte seulement apres les deux gardes Rust. La deposer ici melangerait deux surfaces de test ; la discipline du depot est de la sortir. Le ticket est a filer avec l'evidence de ce plan.

### Assumptions

- L'owner `senara-solutions` est le seul owner legitime pour la boucle. Le defaut applique par `owner_repo()` l'assume deja ; ce plan le rend explicite au lieu de le laisser implicite.
- `wizzard` n'est pas dans la liste. L'arbitrage operateur a nomme quatre depots et ne l'a pas inclus ; le refus etant bruyant et nomme, une omission se verra au premier essai plutot que de passer silencieusement.
- La liste change rarement. C'est l'hypothese qui rend la constante compilee acceptable ; `1053-dispatch-trigger-allowlist-config-constant.md` documente le chemin d'escalade si la frequence de changement montait.

### Sequencing

U1 d'abord — les deux autres unites consomment son predicat. U2 et U3 sont ensuite independantes l'une de l'autre et peuvent etre faites dans n'importe quel ordre.

---

## Implementation Units

### U1. Constante d'allowlist et predicat partage

- **Goal.** Un seul endroit dit quels depots la boucle a le droit de dispatcher, et sait s'enoncer pour etre cite par un refus.
- **Requirements.** R1, R2, R7, R8.
- **Files.** `crates/mika-agent/src/webhook_dispatch.rs`.
- **Approach.** Ajouter une constante de depots pleinement qualifies (`senara-solutions/mika`, `senara-solutions/mika-cloud`, `senara-solutions/mika-skills`, `senara-solutions/mika-platform`) et deux fonctions : un predicat qui repond a « cette paire owner/depot est-elle dispatchable » et un formateur qui rend la liste sous forme lisible pour les messages de refus. Documenter en tete de constante *pourquoi* la liste n'est pas derivee de la presence des repertoires — c'est le contresens que ce ticket corrige, et il doit etre inscrit la ou quelqu'un serait tente de le refaire. Suivre la forme de `is_unauthorized_webhook_dispatch` (visibilite `pub(crate)`, doc-comment portant la raison, annotation de doctrine de garde structurelle pre-classifieur si elle s'applique).
- **Test Scenarios.** Les quatre depots listes sont acceptes sous forme qualifiee. `senara-solutions/control-monitor` et `senara-solutions/claude-pilot` sont refuses. `autre-org/mika` est refuse (R8). Une chaine vide est refusee. Le formateur de liste rend les quatre noms et le test le verifie par contenu, pas par egalite exacte de mise en forme.
- **Verification.** `cargo test -p mika-agent webhook_dispatch`.

### U2. Garde pre-LLM dans le handler ready-label

- **Goal.** Un `ready` pose sur un depot hors liste ne cree aucune tache, ne lance aucun dispatch, laisse une trace nommee, et ne declenche pas le guard qui reclamerait le dispatch.
- **Requirements.** R3, R4, R6, R7.
- **Files.** `crates/mika-agent/src/server/ready_label_handler.rs`.
- **Approach.** Inserer la garde dans `try_handle_ready_label_dispatch` **entre l'etape 2 (parse reussi) et l'etape 3 (token GitHub)** — le plus tot possible apres que le depot est connu, ce qui satisfait « avant toute creation de tache » et evite en prime le `gh issue view` de l'etape 4. Sur refus : emettre `warn!(event = "ready_label_repo_not_dispatchable", repo = ..., num = ..., ...)` en suivant la forme des refus voisins du fichier, avec depot et numero en champs structures ; ecrire un evenement d'audit via `db.log_audit_event` pour la surface operateur, en notant qu'aucun `task_id` n'existe encore a ce point — la cle de cible est la reference `owner/repo#num` ; puis rendre `VerdictAction::Handled` avec un pre-digest commencant par `<ready_label_handler>` (KTD1) qui nomme le depot refuse, cite la liste (R7), et demande explicitement de ne pas dispatcher et de prevenir l'operateur.
- **Test Scenarios.** Un marqueur `senara-solutions/control-monitor#159` produit un `Handled` dont le pre-digest commence par `<ready_label_handler>` et contient le nom du depot refuse et la liste autorisee. Meme chose pour `claude-pilot#119` sous forme courte. Un marqueur `senara-solutions/mika#2046` et un marqueur `mika-cloud#127` ne sont pas interceptes par cette garde (sens positif de R9). Un marqueur `autre-org/mika#1` est refuse (R8). Test de non-regression du desarmement : le pre-digest de refus ne satisfait pas `is_ready_label_dispatch_marker`, verifie par appel direct au predicat plutot que par inspection de chaine.
- **Verification.** `cargo test -p mika-agent ready_label`.

### U3. Garde de frontiere d'outil dans la chaine de dispatch-readiness

- **Goal.** Un appel a `run_claude_pilot` visant un depot hors liste echoue avant que le sous-processus existe, quelle que soit l'origine du tour.
- **Requirements.** R5, R6, R7, R8.
- **Files.** `crates/mika-agent/src/skills/executor.rs`.
- **Approach.** Ajouter un rejet terminal dans `validate_dispatch_readiness`, juste apres le controle `unauthorized_webhook_dispatch` et avant le `get_task` — l'ordre du fichier veut les controles les moins chers d'abord, et celui-ci est un parse de chaine sans acces base. Extraire le depot depuis `tool_input["prompt"]` en suivant la meme forme `[owner/]repo#numero` que `dispatch-lib` accepte ; appliquer l'owner par defaut comme le fait `owner_repo()` pour rester coherent avec U1 ; si le prompt n'a pas cette forme (mode texte libre), ne pas refuser — ce controle porte sur les references de depot, pas sur le texte libre. Sur refus : rendre le JSON d'erreur nomme `repo_not_dispatchable` en suivant exactement la forme du rejet mika#933 voisin (champs `error`, `task_id`, `reason`), la `reason` citant la liste autorisee (R7), et appeler `record_dispatch_rejection` comme les autres rejets terminaux. Ce rejet rejoint la famille des sept rejets structurels documentes en `agent_loop/mod.rs:6447-6453` : il n'est pas recuperable par re-prompt, ce qui est le comportement voulu.
- **Test Scenarios.** Un `prompt` valant `control-monitor#159` est refuse avec `error: "repo_not_dispatchable"` et une raison contenant la liste. `claude-pilot#119` idem. `mika#2046` et `mika-cloud#50` passent ce controle (sens positif de R9). `autre-org/mika#1` est refuse (R8). Un prompt en texte libre contenant un `#` ne declenche pas ce rejet. Le refus enregistre bien la trace via `record_dispatch_rejection`.
- **Verification.** `cargo test -p mika-agent dispatch_readiness`.

---

## Verification Contract

- `cargo build --workspace` — compile.
- `cargo test -p mika-agent` — la suite du crate porteur des trois sites.
- `cargo clippy --workspace --all-targets -- -D warnings` — la porte de qualite du depot.
- `cargo fmt --check`.
- **Test anti-vacuite, exigence explicite du ticket.** La suite doit echouer si l'on supprime la liste et doit echouer si l'on refuse tout. Concretement : chaque unite porte au moins un test du sens positif (`mika`, `mika-cloud` passent) et un test du sens negatif (`control-monitor`, `claude-pilot` refuses). Un correctif qui bloquerait tout passerait un jeu de tests purement negatif — c'est la vacuite que le ticket demande d'exclure.
- **Verification que le correctif est bien dans le diff.** Avant de conclure, verifier que la suite echoue sans la garde : commenter le predicat et constater que les tests negatifs tombent. Discipline `feedback_verify_pipeline_passes_without_the_fix`.

---

## Definition of Done

- Les trois unites sont implementees et leurs tests passent.
- `cargo clippy` et `cargo fmt --check` sont verts.
- Aucun code d'essai ou de tentative abandonnee ne subsiste dans le diff.
- Le ticket separe pour la garde shell de `dispatch-lib.sh` (KTD5) est depose, avec l'evidence de ce plan.
- La PR reference `Closes #2046`.

---

## Acceptance criteria

- [ ] Une allowlist explicite de depots dispatchables existe a un seul endroit du code, et la valeur par defaut pour un depot non liste est le refus (R1).
- [ ] La liste contient exactement `mika`, `mika-cloud`, `mika-skills`, `mika-platform` sous l'owner `senara-solutions` (R2).
- [ ] La garde est appliquee dans `ready_label_handler` avant toute creation de tache (R3).
- [ ] Le refus webhook ne laisse pas l'INTENT_GUARD `webhook_ready_label_dispatch` reclamer un dispatch (R4).
- [ ] La frontiere d'outil `validate_dispatch_readiness` refuse un dispatch vers un depot hors liste, quelle que soit l'origine du tour (R5).
- [ ] Chaque refus emet un evenement nomme — `ready_label_repo_not_dispatchable` cote webhook, `repo_not_dispatchable` cote frontiere d'outil — avec depot et numero en champs structures exploitables (R6).
- [ ] Le message de refus cite la liste autorisee (R7).
- [ ] Un marqueur `autre-org/mika#1` est refuse : la validation porte sur owner+depot, pas sur le basename (R8).
- [ ] Les tests couvrent les deux sens sur chaque couche : `mika#N` et `mika-cloud#N` dispatchent encore, `control-monitor#N` et `claude-pilot#N` sont refuses avec l'evenement nomme (R9).
