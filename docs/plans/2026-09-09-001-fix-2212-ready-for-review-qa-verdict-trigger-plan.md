---
issue: 2212
type: fix
title: "`pull_request.ready_for_review` est routé vers mika-qa mais aucun skill ne le prend — le demi-livrable de mika#1822"
branch: fix/2212/qa-review-routing-aucun-verdict-qa-frais
---

# Plan — #2212 : le verdict QA qui ne vient jamais sur un rescue-draft pipeline-vérifié

> **Re-mesuré le 2026-10-15** au HEAD `27ce7a3b`. Le plan initial datait du 2026-09-09.
> Le défaut **tient intégralement** : le handler n'existe pas, `ready_for_review` est
> toujours routé, l'énumération `qa-review:5` l'omet toujours. Sept points ont bougé et
> sont corrigés ci-dessous : les ancrages de ligne, un fait qui était **faux**
> (enregistrement dans `bundled_skills.rs`), une **contrainte de budget de prompt** qui
> change la phase 2, l'emplacement du test de phase 3 (que `lib.rs` contraint), et la
> phase 0 devenue **non exécutable**. Les mesures sont datées là où elles comptent.

## Problème (mesuré 2026-09-06, re-vérifié sur le code à `27ce7a3b` le 2026-10-15)

Le ticket rapporte deux PR, trois déclenchements, zéro verdict `mika-platform-qa` frais.
Les deux moitiés ont des mécanismes **différents**, et une seule est un défaut.

### Fait 1 — mesuré le 2026-09-09 : zéro review QA sur #2210, une review périmée sur #2202

`gh pr view … --json reviews` (2026-09-09) :

- **PR #2210** (undraft → `ready_for_review`, marker `yes`, CI 20/0, mergée 13:29:51Z) :
  **aucune** review de `mika-platform-qa`. Les trois seules reviews sont `samidarko`
  (10:37:08Z, 12:27:42Z, 12:41:28Z), toutes `COMMENTED`.
- **PR #2202** (commit vide → `synchronize`) : une seule review `mika-platform-qa`, au
  **2026-09-05T19:27:01Z** — le `hold[review]` périmé d'avant la danse marker. Rien après.

Les faits du ticket tiennent. Ce plan ne les re-litige pas ; il nomme les mécanismes.

### Fait 2 — moitié #2202 : `DROP_SYNCHRONIZE_NO_DIFF`, comportement correct, pas un défaut

`crates/mika-gateway/src/github.rs:973` (constante à `audit_events.rs:117`) : sur
`pull_request.synchronize`, le gateway appelle `commits_have_file_changes(before, after)`
et, sur `Ok(false)`, journalise `DROP_SYNCHRONIZE_NO_DIFF` puis `return StatusCode::OK` —
la qa-review n'est jamais dispatchée. Un commit vide ne produit aucun diff : le drop est
exactement ce que la garde #886 promet.

**Ce n'est pas la cause à réparer.** Un `synchronize` vide n'est pas un geste de
re-verdict ; c'est un contournement que l'opérateur a inventé parce que le vrai geste
ne marchait pas. Le vrai geste est le passage draft → ready — la moitié suivante.

### Fait 3 — moitié #2210 : `ready_for_review` est routé, mais aucun skill ne le prend

C'est la cause. Elle est **un demi-livrable de mika#1822**, mesurable dans le code — et
les cinq termes ci-dessous ont été **re-mesurés le 2026-10-15**, tous inchangés :

1. **Le gateway route.** `github.rs:337` :
   ```rust
   ("pull_request", Some("opened" | "synchronize" | "review_requested" | "ready_for_review"))
       => Some("mika-qa"),
   ```
   `crates/mika-gateway/CLAUDE.md:31` dit pourquoi `ready_for_review` a été ajouté
   (mika#1822) : « draft→ready transitions were previously dropped as "not routable",
   **stranding every wip-rescue-shaped PR without an autonomous review** ». Le ticket
   présent est cette même classe, revenue.
2. **La file ne le mange pas.** `crates/mika-agent/src/server/webhook_queue_v2.rs:195` :
   `opened / closed / review_requested / ready_for_review` → `WebhookEventKind::Other`,
   et `coalescing_key(Other) = None` (`:207`) — « never coalesce ». L'événement atteint
   l'agent individuellement.
3. **Le texte livré est reconnaissable.** `format_event_text` (`github.rs`, bras
   `"pull_request"`) produit en première ligne :
   `[GitHub] PR ready_for_review: senara-solutions/mika#<n> — <titre> (branch: <b>)`.
4. **Mais le skill qui devrait agir ne le déclare pas.**
   `skills/bundled/qa-review/system_prompt.md:5` énumère ses déclencheurs :
   > « You are triggered by GitHub webhook events (`pull_request.opened`,
   > `pull_request.synchronize`, `pull_request.review_requested`) routed through the
   > gateway. »

   `ready_for_review` **est absent de cette énumération**. mika#1822 a modifié la table de
   routage du gateway et n'a pas touché le contrat côté agent.
5. **Et aucun handler dédié ne le couvre.** `ls skills/bundled/ | grep qa` au 2026-10-15
   rend exactement `qa-review`, `qa-review-build-callback`, `qa-review-webhook-success` —
   **pas de `qa-review-webhook-ready`**. La seule classe d'événement qui a reçu un
   handler-webhook avec impératif d'action est `check_suite.completed(success)` :
   `skills/bundled/qa-review-webhook-success/` (mika#1711), dont le prompt porte
   « **CRITICAL: DO NOT end your turn without acting** » et un chemin de décision en
   6 étapes. `ready_for_review` n'a pas d'équivalent. Le tableau des handlers mika-qa :

   | skill | déclencheur couvert | impératif d'action |
   |---|---|---|
   | `qa-review` (`always_on = true`) | `opened`, `synchronize`, `review_requested` | non (prompt de revue, pas d'entrée webhook) |
   | `qa-review-webhook-success` | `check_suite.completed(success)` | **oui** (mika#1711) |
   | `qa-review-build-callback` | reprise après `build_mika` | oui |
   | `ready_for_review` | — | **aucun skill** |

6. **Et l'undraft ne relance pas la CI**, donc aucun `check_suite` ne vient rattraper par
   le chemin mika#1711 : le seul événement émis par la transition est celui que personne
   ne prend.

L'activation d'un skill est **structurelle** — `crates/mika-agent/src/skills/matcher.rs:50`
(`build_matcher_regex`) apparie les `[triggers] keywords` du manifeste contre le message ;
`index.rs:1554` refuse même un skill « not always_on and no trigger keywords » (« skill will
never activate »). Un handler dédié avec le mot-clé `ready_for_review` s'active donc par
appariement moteur, pas par jugement du LLM. C'est le même levier que mika#1711, dont on
sait empiriquement qu'il tient (les reviews sur CI verte partent).

### Fait 4 — la garde marker n'est pas en cause

`qa-review/system_prompt.md:114-136` (re-mesuré ; `:110-131` dans le plan initial), item 4 :
la PR est réputée pipeline-vérifiée si `marker == yes` **OU** `isDraft == false` **OU**
aucun marker. #2202 et #2210 satisfaisaient les deux premières. Une qa-review **fraîche**
aurait procédé. Le défaut est strictement en amont : la revue fraîche n'a jamais lieu.

### Fait 5 — nouveau, mesuré le 2026-10-15 : le prompt `qa-review` est à 78 octets de son gate

C'est la découverte qui change la phase 2, et elle n'existait pas dans le plan initial.

- `wc -c skills/bundled/qa-review/system_prompt.md` → **77 259 octets**.
- `skills/bundled/qa-review/skill.toml:108` → `max_prompt_size = 81408`.
- `crates/mika-agent/tests/bundled_skills_load.rs:141` → `if ratio >= 0.95 { … panic!() }`.

Le gate **panique**, il n'avertit pas : le nom
`bundled_skills_approaching_max_prompt_size_warns` dit « warns », le corps dit `panic!`.
Le seuil est à `0.95 × 81408 = 77 337` octets. **Marge disponible : 78 octets.**

L'ajout d'AC4 (`, \`pull_request.ready_for_review\``) coûte ~34 octets et tiendrait de
justesse — mais la *citation* mika#1822/mika#2212 que le plan initial demandait dans le
prompt ne tient pas, et 44 octets de marge résiduelle ne sont pas un état livrable.
La remédiation est tranchée en **D-F**.

### Fait 6 — nouveau, mesuré le 2026-10-15 : le plan initial affirmait un enregistrement qui n'existe pas

Le plan initial exigeait d'enregistrer le handler « dans
`crates/mika-agent/src/bundled_skills.rs` selon le motif `BundledSkill` existant ».
**C'est faux**, et l'implémenter ferait perdre des tours à chercher un motif inapplicable :

- `grep -n 'qa-review-webhook-success' crates/mika-agent/src/bundled_skills.rs` → **vide**.
  Le précédent exact n'y est pas enregistré.
- `bundled_skills.rs:186` déclare `BUNDLED_SKILLS` — les skills **legacy** codés en dur
  (`tmux`, `shell-exec`, `github`, …) — puis `:210-213` les **fusionne** avec `ENTRIES`,
  généré par `build.rs` en marchant `skills/bundled/` (`build.rs:120`).

Un skill posé sous `skills/bundled/` est donc découvert **à la compilation**, sans ligne à
écrire dans `bundled_skills.rs`. Seule l'allowlist reste manuelle (phase 1).

### Incertitude nommée — et pourquoi la phase 0 initiale n'est plus exécutable (2026-10-15)

La table `tasks` porte une ligne `mika-qa` du 2026-09-06T10:30:48 :
« QA review mika#2210 — build verification for behavioral ACs (logging.rs double-write fix,
mika#2195) », `trigger_type = manual`, `status = completed` — et aucune review n'a été
postée. Son libellé et son `trigger_type` désignent une vérification de build demandée à la
main sur le **contenu** de #2210, pas une qa-review déclenchée par le webhook
`ready_for_review`. C'est l'interprétation retenue, et **elle reste non prouvée**.

Le plan initial faisait de sa levée une « phase 0 ». Mesuré aujourd'hui, **l'évidence est
hors d'atteinte du pilote d'implémentation** :

- `ls ~/.mika/logs/` → le répertoire est **absent** du sandbox de dispatch.
- `ls ~/.mika/data/mika.db` → **absent** (seuls `~/.mika/data` et `~/.mika/state` existent).
- Et les événements visés datent du 2026-09-06, soit **plus de cinq semaines** — au-delà de
  toute rétention de log raisonnable, même sur un hôte où ils seraient lisibles.

Un AC qui exige une mesure inaccessible ne produit pas une mesure : il produit un pilote
bloqué ou une affirmation fabriquée. La phase 0 est donc **retirée du périmètre
d'implémentation** (D-G), et c'est sans coût pour le livrable : **le fait 3 est lu dans le
code, pas dans les logs**, et il a été re-mesuré ce jour. L'incertitude porte sur
*pourquoi* le tour de #2210 n'a rien posté — pas sur *si* `ready_for_review` est sans
preneur. Le ticket de suite d'observabilité (D-D) est précisément la couche qui rendra
cette question mesurable la prochaine fois, au lieu de la laisser à la rétention.

## Décisions de grooming (tranchées)

### D-A — Le geste canonique de re-verdict est l'undraft, pas le commit vide

`DROP_SYNCHRONIZE_NO_DIFF` **n'est pas modifié**. Réparer #2202 en faisant passer les
`synchronize` sans diff rendrait re-dispatchable tout amend de trailer, ce que la garde #886
existe précisément pour empêcher. La moitié #2202 se referme **par** la moitié #2210 :
une fois `ready_for_review` pris en charge, l'opérateur a un geste qui marche et n'a plus
besoin d'inventer le commit vide.

### D-B — Un handler dédié, pas une phrase de plus dans `qa-review`

Ajouter `ready_for_review` à l'énumération de `qa-review/system_prompt.md:5` est
**nécessaire mais insuffisant** : ce prompt est une procédure de revue, pas une entrée
webhook, et il ne porte aucun impératif « n'arrête pas le tour sans agir ». C'est de
l'enforcement par prompt au niveau substrat, la classe que ce dépôt a déjà mesurée comme
défaillante. Le livrable structurel est un handler dédié — `qa-review-webhook-ready` —
dont l'**activation** est un appariement moteur (`matcher.rs:50`), sur le modèle exact et
déjà éprouvé de `qa-review-webhook-success` (mika#1711).

### D-C — Une garde de cohérence pour que la classe ne revienne pas

mika#1822 a changé la table de routage sans changer le contrat côté agent, et rien n'a
échoué. C'est le défaut fondateur, et il est plus général que ce ticket. Le plan ajoute un
test qui échoue si une action `pull_request` routée vers `mika-qa` par `route_event`
n'est couverte par aucun handler.

**Emplacement — tranché, et plus précisément que dans le plan initial.** Mesuré :
`crates/mika-gateway/Cargo.toml` dépend de `mika-common` et `mika-a2a`, **pas** de
`mika-agent` ; `crates/mika-agent/Cargo.toml` ne dépend pas de `mika-gateway`. Aucune crate
ne voit aujourd'hui à la fois `route_event` et les manifestes bundled. La résolution est de
ne pas créer la dépendance : le test vit dans `mika-gateway` et lit les manifestes **depuis
le disque**, par un chemin relatif à `env!("CARGO_MANIFEST_DIR")` vers `skills/bundled/` —
motif déjà en service à `crates/mika-agent/tests/bundled_skills_load.rs:33`
(`bundled_skills_dir()`). Ce répertoire est versionné ; `include_str!` n'en est qu'un
consommateur. Écrire le test côté `mika-agent` imposerait d'ajouter `mika-gateway` aux
dépendances de `mika-agent` pour un seul test : refusé.

**Test unitaire in-crate, pas test d'intégration — contrainte mesurée le 2026-10-15.**
`crates/mika-gateway/src/lib.rs` n'expose que `pub mod voice` et porte l'interdiction
explicite : « **Do NOT add unrelated modules here.** If a module wants a library surface, it
must have a documented external consumer ». Un test sous `tests/` ne pourrait donc appeler
`route_event` (`github.rs:320`, `pub fn`) qu'en exposant `pub mod github` — en violation
directe de cette consigne. Le test est donc un **test unitaire dans le `mod tests` de
`crates/mika-gateway/src/github.rs`**, où `route_event` est appelable sans toucher à
`lib.rs`. `env!("CARGO_MANIFEST_DIR")` fonctionne identiquement depuis `src/`.

**Dépendance de test :** le terme T1 lit un `skill.toml`. `toml` n'est pas dans les
dev-dependencies de `mika-gateway` mais **est** une dépendance de workspace
(`Cargo.toml:41`, `toml = "1.1"`) : ajouter `toml = { workspace = true }` sous
`[dev-dependencies]` suffit. On parse le manifeste plutôt que d'y chercher une sous-chaîne,
parce qu'une sous-chaîne brute serait satisfaite par un **commentaire** mentionnant
l'action — un faux vert exactement de la nature que cette garde existe pour exclure.

**Terme de couverture (disjonction à deux termes, tous deux lus sur disque) :**
une action `a` est couverte si

- **T1** — un `skills/bundled/*/skill.toml` de la famille qa-review déclare `a` dans
  `[triggers] keywords` (lu par parse TOML, pas par recherche de texte) ; **ou**
- **T2** — `skills/bundled/qa-review/system_prompt.md` contient la sous-chaîne littérale
  `pull_request.<a>`.

T2 existe parce que `qa-review` est `always_on = true` : `opened`, `synchronize` et
`review_requested` ne sont couverts par aucun mot-clé de manifeste et n'ont pas à l'être —
ils sont couverts par l'énumération de `qa-review:5`, qui est le contrat qu'ils ont
réellement. Une recherche de sous-chaîne littérale sur `pull_request.opened` est mécanique,
pas de l'analyse de prose : la forme est exacte. C'est précisément le terme qui manquait
pour `ready_for_review` et dont l'absence n'a rien fait échouer.

L'ensemble des actions est **lu depuis `route_event`** en itérant sur une liste d'actions
candidates et en ne retenant que celles qui rendent `Some("mika-qa")` — jamais recopié en
dur : c'est le recopiage qui laisserait passer la prochaine divergence. Deux tests existants
de `github.rs` (`:1837`, `:1974`) itèrent sur des listes d'actions **en dur** ; ils restent
inchangés — leur objet est l'indépendance à l'auteur et l'absence de fan-out, pas la
couverture, et c'est bien pour cela qu'ils n'ont rien vu en mika#1822.

### D-D — Hors périmètre, avec ticket de suite nommé

- **`pull_request.edited`** (bascule du marker `no → yes` **sans** undraft) : `route_event`
  ne route pas `edited` du tout (`_ => None`). Un opérateur qui édite seulement le corps
  n'émet aucun événement pris. Non traité ici : l'undraft est le geste canonique (et
  `isDraft == false` suffit à la garde marker, item 4), et router `edited` ouvre un
  robinet d'événements bien plus large qui mérite son propre arbitrage.
  **Ticket de suite à ficher** après merge, avec l'évidence de ce plan.
- **Observabilité « tour qa-review terminé sans review postée »** : un événement d'audit
  quand un tour mika-qa déclenché par un webhook éligible se termine sans appel
  `run_gh pr review`. C'est la couche de détection qui aurait rendu cette classe visible
  sans trois mesures à la main — et c'est ce qui aurait permis de trancher l'incertitude de
  #2210 **avant** que la rétention de log ne l'emporte (voir la section Incertitude).
  Surface différente (`agent_loop`), classe différente (dispatch-FAIL en général).
  **Ticket de suite à ficher.**
- **`DROP_REVIEWER_FILTER`** (mika#1655) : inchangé, et sans effet ici — #2210 était
  `ready_for_review`, pas `review_requested`.

### D-E — `isDraft == true` et la garde marker répondent à deux questions différentes

Le handler saute sur `isDraft == true` ; la garde marker de `qa-review:114-136` item 4
répute une PR vérifiée si `marker == yes` **même en draft**. Ces deux règles ne se
contredisent pas — elles portent sur des questions distinctes, et l'incident lui-même le
montre :

- **Éligibilité au dispatch** (question du handler) : *cette transition draft→ready
  justifie-t-elle de dépenser un tour de revue maintenant ?* Si la PR est redevenue draft
  entre l'émission de l'événement et son traitement, l'opérateur a **retiré** son signal de
  disponibilité. Sauter est la bonne réponse, et c'est déjà la règle du précédent :
  `qa-review-webhook-success` step 2 exige `draft: false`.
- **Traitement du boilerplate rescue** (question de `qa-review`) : *une fois la revue
  engagée, la boilerplate « Auto-rescued PR » doit-elle bloquer le verdict ?* L'item 4 dit
  non quand le marker est `yes`. Il ne dit rien sur l'opportunité de déclencher une revue.

**Règle tranchée :** l'éligibilité au dispatch appartient au handler et le draft y est
disqualifiant ; le sort de la boilerplate rescue appartient à `qa-review` et le marker y
est décisif. Le cas rescue-draft `marker: yes` + re-draft n'est donc pas un verdict perdu :
il est un verdict **non déclenché**, et le geste qui le déclenche reste l'undraft — celui
que ce ticket rend enfin opérant. Aucune divergence n'est laissée implicite.

### D-F — Le budget de prompt : relever le plafond au plafond dur, et mettre la citation dans le manifeste (fait 5)

78 octets de marge ne sont pas un état livrable, et le gate **panique** plutôt que
d'avertir. Deux gestes, tranchés :

1. **`max_prompt_size` passe de `81408` à `81920`** — soit exactement
   `MAX_PROMPT_SIZE_CEILING` (`crates/mika-agent/src/skills/index.rs:27`, 80 KB), que
   `effective_prompt_limit` (`:42`) applique par un `.min()`. Déclarer plus haut n'aurait
   aucun effet ; déclarer `81920` est donc la totalité de la marge disponible par ce geste.
   Le seuil de 95 % passe de 77 337 à **77 824** octets, portant la marge de 78 à
   **565 octets** après l'ajout d'AC4.
2. **La citation mika#1822/mika#2212 va dans `skill.toml`, pas dans le prompt.** Le
   manifeste pèse 9 512 octets contre un `MAX_SKILL_TOML_SIZE` de 64 KB (`index.rs:16`) :
   la place y est abondante, et c'est le fichier qui porte déjà les commentaires de
   budget. Le prompt ne reçoit que l'ajout mécanique, `, \`pull_request.ready_for_review\``
   — soit le terme T2 exigé par AC3, et rien de plus.

**Ce que ce geste ne fait pas :** il ne crée pas de réserve pour la suite. Après ce vol,
`qa-review` est **au plafond dur**, et le prochain ajout à ce prompt n'aura plus de
remédiation par relèvement — il faudra trimmer ou sharder via `[dependencies]`, les deux
autres options que le gate nomme (`bundled_skills_load.rs:158-161`). Ce fait est consigné
ici, et le commentaire ajouté au manifeste le dit à l'endroit où le prochain éditeur le
lira. Sharder `qa-review` est **hors périmètre** de ce vol : c'est une refonte du plus gros
prompt du dépôt, sans rapport avec le défaut de routage, et la borne par vol ne la tient pas.

### D-G — La phase 0 sort du périmètre d'implémentation, et AC6 mesure ce qui est mesurable

Le plan initial faisait de la levée d'incertitude une phase 0 « non bloquante » dont AC6
exigeait néanmoins une disposition écrite en commentaire sur le ticket. Mesuré le
2026-10-15, l'évidence est inaccessible (voir la section Incertitude : pas de
`~/.mika/logs/`, pas de `mika.db` dans le sandbox, et cinq semaines écoulées).

**Tranché :** la phase 0 est **supprimée**. AC6 ne demande plus une disposition sur une
mesure impossible ; il demande ce qui est vérifiable dans le diff — que l'incertitude soit
consignée comme non tranchable, avec sa raison mesurée, et que le ticket de suite
d'observabilité (D-D) soit nommé comme la voie qui la rendra tranchable. Un AC doit être
satisfiable par le vol qui le porte, sinon il n'est pas un critère mais un vœu.

## Phases

### Phase 1 — Le handler `qa-review-webhook-ready`

Nouveau skill bundled `skills/bundled/qa-review-webhook-ready/`, calqué sur
`qa-review-webhook-success/`.

**`skill.toml`** : `always_on = false`, `dependencies = ["qa-review"]`, et

```toml
[triggers]
keywords = ["ready_for_review", "PR ready_for_review"]
```

**Mots-clés — tranché, sur mesure du moteur d'appariement.**
`crates/mika-agent/src/skills/matcher.rs:50` (`build_matcher_regex`) et `:119`
(`message_lower = user_message.to_lowercase()`) apparient **avec limites de mots et sans
sensibilité à la casse** ; les tests `:714` (`test_word_boundary_bare_bigram_does_not_collide_on_prose`)
et `:796` (`test_word_boundary_multiword_keyword_requires_adjacent_tokens`) fixent cette
sémantique. Conséquence directe : le candidat `"ready for review"` (avec espaces) **fire sur
de la prose ordinaire** — « this PR is ready for review » suffit — et activerait le handler
sur des tours conversationnels sans rapport. Il est **retiré**. Le candidat `"undraft"` est
également retiré : il n'apparaît dans aucun texte d'événement produit par
`format_event_text`, et sa seule fonction serait d'ouvrir une porte conversationnelle que
personne n'a demandée. Restent les deux formes exactes qui collent à la première ligne
réellement émise : `ready_for_review` et `PR ready_for_review`.

**Enregistrement — l'allowlist, et elle seule (fait 6).** Le skill est découvert à la
compilation par `build.rs:120` dès qu'il est posé sous `skills/bundled/` : **aucune ligne à
écrire dans `bundled_skills.rs`** (le plan initial l'exigeait à tort ; `qa-review-webhook-success`
n'y figure pas). Ce qui reste manuel est l'allowlist, à **deux** emplacements de
`crates/mika-agent/src/well_known_agents.rs` — re-mesurés le 2026-10-15 : **`:323`** (JSON
d'identité de `mika-qa`) et **`:2472`** (test de l'allowlist). Le plan initial citait
`:225`/`:1968`, désormais périmés. Un oubli de `:323` produit un skill qui existe, se seede
dans la bibliothèque canonique, et n'est **jamais** lié sous
`{global_home}/agents/mika-qa/skills/` — invisible, sans échec.

**`system_prompt.md`** : point d'entrée webhook, impératif « ne termine pas le tour sans
agir », et chemin de décision aligné sur celui de `qa-review-webhook-success` :

1. Corréler le numéro de PR depuis la première ligne de l'événement.
2. Sauter si hors périmètre (dépôt non révisable, auteur humain avec relecteur désigné).
3. Sauter si `isDraft == true` — la PR est redevenue draft entre l'événement et le
   traitement (voir D-E pour pourquoi cela ne contredit pas la garde marker).
4. Sauter si une review de `mika-platform-qa` existe déjà **au SHA de tête courant**
   (`commit_id == pr.headRefOid`) — la déduplication au SHA, pas au numéro de PR.
   C'est ce qui empêche l'undraft répété de produire des verdicts en double, et c'est
   ce qui fait que la review périmée de #2202 (2026-09-05, autre SHA) **ne** supprime
   **pas** la revue fraîche.
5. Sinon, appeler `qa-review` — qui possède seule le diff, la vérification plan-AC, la
   vérification de build et l'émission du verdict. Le handler ne duplique aucune de ces
   règles.
6. Discipline de tour : sur impossibilité de procéder, `send_message` à l'opérateur avec
   la raison précise, jamais de fin de tour silencieuse.

Le handler **ne réimplémente pas** la garde marker rescue : elle vit dans
`qa-review/system_prompt.md:114-136` et s'applique telle quelle une fois `qa-review`
appelée. Un rescue-draft passé non-draft y est réputé vérifié par l'item 4.

### Phase 2 — Fermer l'énumération de `qa-review`, sous la contrainte de budget (D-F)

1. `skills/bundled/qa-review/system_prompt.md:5` : ajouter `pull_request.ready_for_review`
   à la liste des déclencheurs. **Ajout mécanique seul** — la citation va au manifeste
   (D-F point 2). Cette ligne est le contrat lu par le modèle quand `qa-review` est appelée
   depuis le handler, et c'est le terme T2 qu'AC3 exige ; la laisser incomplète maintient la
   contradiction qui a produit l'incident.
2. `skills/bundled/qa-review/skill.toml:108` : `max_prompt_size` de `81408` à `81920`
   (= `MAX_PROMPT_SIZE_CEILING`), avec le commentaire qui cite mika#1822/mika#2212, donne
   la mesure du jour (77 259 octets, marge 78 avant / 565 après) et **nomme explicitement
   que la réserve par relèvement est désormais épuisée** — le prochain éditeur doit trimmer
   ou sharder.
3. Vérifier en fin de phase que `bundled_skills_approaching_max_prompt_size_warns` passe.
   C'est un `panic!` à `ratio >= 0.95`, pas un avertissement : le mesurer, ne pas le
   supposer.

### Phase 3 — La garde de cohérence routage ↔ handlers (D-C)

Test **unitaire** dans le `mod tests` de `crates/mika-gateway/src/github.rs` (emplacement
tranché en D-C : ni `tests/`, ni `lib.rs`, aucune dépendance de crate nouvelle), plus
`toml = { workspace = true }` sous `[dev-dependencies]` de `crates/mika-gateway/Cargo.toml` :

1. Construire l'ensemble `A` des actions `pull_request` routées vers `mika-qa`, en itérant
   sur une liste d'actions candidates et en filtrant sur
   `route_event("pull_request", Some(a), None) == Some("mika-qa")`. `A` n'est jamais
   recopié en dur.
2. Charger, depuis `env!("CARGO_MANIFEST_DIR")` + chemin relatif vers `skills/bundled/`
   (motif de `bundled_skills_load.rs:33`) : les `skill.toml` de la famille qa-review,
   **parsés en TOML** pour lire `[triggers] keywords` (T1 — pas de recherche de sous-chaîne,
   qu'un commentaire satisferait), et le texte de `qa-review/system_prompt.md` (T2).
3. Pour chaque `a ∈ A`, affirmer T1 ∨ T2 (définis en D-C). Message d'échec nommant l'action
   découverte et les deux termes manquants, pour que la prochaine divergence se lise sans
   enquête.
4. Suivre le précédent de `bundled_skills_load.rs:93-97` pour l'absence du répertoire :
   échec dur avec « repo layout changed? », jamais un skip silencieux — un test qui se tait
   quand il ne trouve pas sa matière est un test qui ne garde rien.

### Phase 4 — Tests

- `route_event` : `ready_for_review → mika-qa` (déjà couvert, `github.rs:1812` et `:1837`) ;
  ne pas régresser.
- Manifeste : `qa-review-webhook-ready` déclare des mots-clés (sinon `index.rs:1554` le
  déclare « never activate »), déclare `qa-review` en dépendance, et passe
  `make verify-bundled-skills`.
- Cohérence phase 3 : contrôle rouge-avant **terme par terme**, protocole en trois mesures
  détaillé sous `## Fire-Disposition`. Ne pas se contenter d'une seule neutralisation : la
  couverture est une disjonction, un seul terme retiré laisse l'assertion satisfaite.
- Allowlist : `qa-review-webhook-ready` présent aux deux emplacements de
  `well_known_agents.rs` (`:323`, `:2472`).
- Budget : `cargo test -p mika-agent --test bundled_skills_load` — les deux gates de taille.
- `cargo test` (pas seulement `clippy`) sur `mika-agent` et `mika-gateway`.

### Phase 5 — Vérification de bout en bout

Sur une PR de test dans le dépôt, **les deux branches d'AC2 dans la même session** :

1. Ouvrir en draft, marquer non-draft → une review `mika-platform-qa` fraîche est postée au
   SHA de tête. (Contrôle positif.)
2. Re-draft puis re-undraft **sans nouveau commit** → aucun second verdict, la review du
   SHA courant existant déjà. (Contrôle négatif de la déduplication.)
3. Pousser un commit, puis undraft → un verdict frais au nouveau SHA, la review du SHA
   antérieur ne le supprimant pas. (Le cas #2202.)

La mesure porte sur la review postée — source de vérité selon
`qa-review/system_prompt.md:47` — jamais sur un log. Un contrôle positif seul ne
distinguerait pas « le handler marche » de « le handler poste toujours ».

**Dépendance de déploiement :** un changement sous `skills/bundled/` n'atteint un agent que
par `make deploy` (rebuild → seed → read, `CLAUDE.md § Deploying a bundled-skill change`).
La phase 5 n'est donc pas exécutable depuis le worktree de dispatch ; elle appartient à la
vérification post-merge de l'opérateur, et AC5 le dit.

## Acceptance criteria

- **AC1** — Le handler `qa-review-webhook-ready` existe, s'active **par appariement moteur**
  sur la première ligne `[GitHub] PR ready_for_review: …`, déclare exactement les mots-clés
  `["ready_for_review", "PR ready_for_review"]`, dépend de `qa-review`, et son prompt porte
  l'impératif de ne pas terminer le tour sans agir.
  *Vérification :* `skill.toml` + `system_prompt.md` dans le diff ;
  `make verify-bundled-skills` passe ; le skill est présent aux deux emplacements de
  `well_known_agents.rs` (`:323` identité `mika-qa`, `:2472` test d'allowlist). **Aucune
  ligne dans `bundled_skills.rs`** — la découverte est faite par `build.rs` (fait 6).

- **AC2** — Déduplication au SHA de tête. Un second `ready_for_review` sur le même
  `headRefOid`, alors qu'une review `mika-platform-qa` existe déjà à ce SHA, ne produit pas
  de second verdict ; une review à un SHA **antérieur** (cas #2202, review du 2026-09-05) ne
  supprime **pas** la revue fraîche.
  *Vérification :* le chemin de décision du handler l'énonce (étape 4, `commit_id ==
  pr.headRefOid`) ; la phase 5 exerce les deux branches après déploiement.

- **AC3** — Le test de cohérence routage ↔ handlers passe : pour chaque action `a` telle que
  `route_event("pull_request", Some(a), None) == Some("mika-qa")`, `a` est couverte par T1
  (mot-clé lu par parse TOML d'un `skill.toml` de la famille qa-review) ou T2 (sous-chaîne
  littérale `pull_request.<a>` dans `qa-review/system_prompt.md`). L'ensemble des actions est
  dérivé de `route_event`, jamais recopié en dur. Le test est **unitaire, dans
  `src/github.rs`** — `lib.rs` n'est pas modifié (D-C).
  *Vérification :* rouge-avant **terme par terme** (voir Fire-Disposition), vert après ;
  `git diff` ne touche pas `crates/mika-gateway/src/lib.rs`.

- **AC4** — `qa-review/system_prompt.md:5` énumère `pull_request.ready_for_review` — ce qui
  est aussi le terme T2 qu'AC3 exige pour cette action.
  *Vérification :* lecture du fichier dans le diff ; AC3 échoue si la ligne est retirée.

- **AC5** — Bout en bout : une PR non-draft, marker `rescue-pipeline-verified: yes`, CI
  verte, passée de draft à ready, reçoit une review **postée** par `mika-platform-qa` au SHA
  de tête courant.
  *Vérification :* phase 5, sur la review GitHub — source de vérité selon
  `qa-review/system_prompt.md:47` — et non sur un log. **Post-merge et post-`make deploy`** :
  un changement bundled n'atteint aucun agent avant le rebuild, donc cet AC ne peut pas être
  satisfait dans le worktree de dispatch et ne bloque pas la PR.

- **AC6** — Le budget de prompt de `qa-review` tient, et son épuisement est consigné :
  `max_prompt_size` vaut `81920` (le plafond dur), le commentaire du manifeste cite
  mika#2212 avec la mesure du jour et nomme que la réserve par relèvement est épuisée, et
  `bundled_skills_approaching_max_prompt_size_warns` **passe** (c'est un `panic!` à 95 %, pas
  un avertissement).
  *Vérification :* `cargo test -p mika-agent --test bundled_skills_load` vert ; diff du
  manifeste.

- **AC7** — L'incertitude de #2210 (pourquoi le tour n'a rien posté) est consignée comme
  **non tranchable**, avec sa raison mesurée — évidence inaccessible au pilote
  (`~/.mika/logs/` absent, `mika.db` absent du sandbox) et plus de cinq semaines de
  rétention écoulées — et le ticket de suite d'observabilité (D-D) est nommé comme la voie
  qui la rendra tranchable.
  *Vérification :* la section « Incertitude nommée » du plan et D-G ; rien à mesurer hors du
  diff (D-G : un AC doit être satisfiable par le vol qui le porte).

- **AC8** — `DROP_SYNCHRONIZE_NO_DIFF` (`github.rs:973`, `audit_events.rs:117`) et
  `DROP_REVIEWER_FILTER` (mika#1655) sont **inchangés**, la garde marker de
  `qa-review:114-136` est **inchangée**, et `crates/mika-gateway/src/lib.rs` est
  **inchangé**.
  *Vérification :* absents du diff.

## Fire-Disposition

Les détecteurs livrés par ce plan, et ce qui doit arriver quand ils font feu.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est vide.** Le
détecteur de la phase 3 est mesuré **rouge-avant puis vert** sans aucune exception, parce
que la seule violation existante (`ready_for_review` non couvert) est **réparée par ce même
vol** — c'est le livrable, pas une dette à annoter. Le détecteur atterrit donc **armé, sur
un dépôt propre, sans entrée d'allowlist** : il n'y a aucune donnée à exempter. Si
l'implémentation découvre une action `pull_request` routée vers `mika-qa` **autre** que les
quatre connues et non couverte, elle est hors du périmètre mesuré de ce plan : l'option
devient (c) halte-et-remontée à l'opérateur pour cadrage, jamais une exception silencieuse
ni un retrait de l'action de la table de routage.

| Détecteur | Nature | Disposition |
|---|---|---|
| Test de cohérence routage ↔ handlers (phase 3, AC3) | **Gate CI bloquant, armé, sans exception** | Un feu signifie qu'une action `pull_request` est routée vers `mika-qa` sans handler ni énumération — exactement la classe mika#1822. Le feu **bloque le merge** ; la remédiation est d'ajouter le terme manquant, **jamais** de retirer l'action de la table de routage pour faire taire le test. |
| `make verify-bundled-skills` sur le nouveau manifeste (AC1) | Gate CI bloquant (existant) | Feu = manifeste invalide ou skill « never activate » (`index.rs:1554`). Bloquant, remédiation dans le manifeste. |
| Test d'allowlist `well_known_agents.rs:2472` (AC1) | Gate CI bloquant (existant) | Feu = le skill n'est pas dans l'allowlist `mika-qa`. Bloquant. |
| `bundled_skills_approaching_max_prompt_size_warns` (phase 2, AC6) | Gate CI bloquant (existant) — **`panic!`, pas un warn** | Mesuré : 77 259 / 81 408 = 94,9 %, marge **78 octets**. Remédiation retenue : relever à `81920` = plafond dur (D-F), marge 565 après AC4. **Pas** le retrait de l'énumération — c'est elle le terme T2. Après ce vol la réserve par relèvement est épuisée : le prochain feu se remédie par trim ou shard. |

**Rouge-avant, terme par terme (AC3).** La couverture d'AC3 est une **disjonction** T1 ∨ T2 :
neutraliser un seul terme ne prouve rien, puisque l'autre satisfait encore l'assertion. Le
contrôle rouge exige donc **deux mesures séparées**, chacune avec l'autre terme neutralisé :

1. Retirer `ready_for_review` des mots-clés du handler, `qa-review:5` neutralisée → rouge.
2. Retirer `pull_request.ready_for_review` de `qa-review:5`, mot-clé du handler neutralisé
   → rouge.
3. Les deux termes en place → vert.

Sans ces deux mesures, un test vert n'établit pas que la garde tient : il peut n'avoir
jamais évalué que le terme survivant.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/qa-review-webhook-ready/skill.toml` | 24 |
| `skills/bundled/qa-review-webhook-ready/system_prompt.md` | 58 |
| `skills/bundled/qa-review/system_prompt.md` (ligne 5, ajout mécanique) | 1 |
| `skills/bundled/qa-review/skill.toml` (`max_prompt_size` + commentaire D-F) | 12 |
| `crates/mika-agent/src/well_known_agents.rs` (`:323`, `:2472`) | 2 |
| test de cohérence routage ↔ handlers (`crates/mika-gateway/src/github.rs`, `mod tests`) | 95 |
| `crates/mika-gateway/Cargo.toml` (dev-dep `toml`) | 3 |

Total estimé : 195 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Un seul vol, et voici pourquoi il tient.** 195 estimées contre une borne par vol de 243 et
un seuil par PR de 1 000 : la plus serrée des deux est la borne par vol, et le total passe
dessous avec ~20 % de marge. Le volume est dominé par un seul poste de code Rust (95 lignes,
le test de phase 3) ; le reste est du prompt et du manifeste, où les lignes sont courtes et
sans boucle de compilation-correction. Trois facteurs de risque sont déjà neutralisés par le
grooming plutôt que laissés au vol : l'emplacement du test est tranché (pas d'exploration de
`lib.rs`), les deux sites d'allowlist sont re-mesurés au HEAD (`:323`, `:2472` — pas de
chasse), et la contrainte de budget de prompt est chiffrée avec sa remédiation (pas de
découverte d'un `panic!` à 95 % en fin de pipeline). La phase 5, qui exige `make deploy`,
est explicitement post-merge (AC5) et ne consomme aucun tour du vol.

## Hors périmètre

- Router `pull_request.edited` (bascule du marker sans undraft) — ticket de suite (D-D).
- Événement d'audit « tour qa-review terminé sans review postée » — ticket de suite (D-D),
  et la voie qui rendra tranchable l'incertitude de #2210 (D-G).
- Sharder ou trimmer `qa-review/system_prompt.md` — le plus gros prompt du dépôt, sans
  rapport avec le défaut de routage, et au-delà de la borne par vol (D-F).
- Toute modification de `DROP_SYNCHRONIZE_NO_DIFF` (D-A) ou de `DROP_REVIEWER_FILTER`.
- La garde marker rescue de `qa-review/system_prompt.md:114-136` — correcte, non touchée
  (fait 4).
- Toute modification de `crates/mika-gateway/src/lib.rs` — son interdiction d'ajout de
  module est respectée par construction (D-C, AC8).
- La levée de l'incertitude #2210 par les logs — évidence inaccessible et périmée (D-G).
- La classe rescue-draft en amont (mika#2211, CLOSED) et le double-logging (mika#2195,
  CLOSED) — livrés, hors de ce diff.
