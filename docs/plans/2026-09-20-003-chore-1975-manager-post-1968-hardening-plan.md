# Plan — mika#1975 : la sonde d'auth du manager est bornée, sa garde ne peut plus s'empoisonner, et son classificateur ne lit plus ses propres arguments

- **Ticket :** senara-solutions/mika#1975
- **Type :** chore (substrat — durcissement post-mika#1968 du `milestone_manager`)
- **Date :** 2026-09-20
- **Branche :** `chore/1975/manager-post-1968-hardening-pass-timeout`

---

## Goal Capsule

**Objectif.** Trois durcissements du `milestone_manager` issus des revues
adversariale et testing de PR#1973 : la sonde d'auth de démarrage est bornée
dans le temps, la garde anti-double-spawn ne peut plus laisser un état
inutilisable derrière elle, et le classificateur d'erreurs de cycle cesse de
classer sur du texte que lui-même a fabriqué.

**Moyen.** Un budget nommé sur la sonde, une garde sans verrou, et un
classificateur qui lit le `stderr` typé plutôt que la ligne de commande.

**Condition d'arrêt.** Le classificateur alimente l'alarme d'auth persistante
(mika#2013 volet B, élargie par mika#2063). Toute modification qui retire un
vrai positif rouvre l'aveuglement silencieux que ces deux tickets ont fermé.
Le resserrement ne peut donc retirer **que** des faux positifs — invariant D7,
épinglé par test.

---

### Ce que le plan rectifie du ticket, et c'est le premier livrable

#### R1 — Le démarrage n'est pas bloqué. C'est la cadence qui l'est.

Le ticket écrit : *« sans timeout, un GitHub API pendant pourrait wedge le boot
indéfiniment »*. Le code dit autre chose.

`spawn_manager_cycle_task` rend un `JoinHandle` **immédiatement**
(`tokio::spawn` à `spawn.rs:253`), et `run_server` ne l'attend jamais
(`server/mod.rs:1460-1471` : le handle est stocké dans `manager_handle` et
seulement `abort`é à l'arrêt). Or `verify_gh_auth` est appelé **à l'intérieur**
de la tâche spawnée (`spawn.rs:280`). Un `gh` pendant ne peut donc pas retarder
le démarrage du serveur d'une milliseconde.

Ce qu'il bloque est la **boucle de cadence, avant son premier tick** :
`tokio::time::interval` n'est construit qu'à `spawn.rs:337-338`, après la sonde.
Un `gh` pendant ⇒ **zéro cycle, zéro rapport, pour toujours.** C'est plus grave
que ce que le ticket décrit, pas moins : un boot retardé se voit ; une cadence
qui n'a jamais démarré ne se voit pas.

**Et le blocage est sans borne d'aucune sorte.** `ProcessGhRunner::run`
(`reader.rs:48-65`) n'a **aucun timeout** : `cmd.output().await` sur un `gh`
dont la connexion TCP est avalée attend indéfiniment. `kill_on_drop(true)` ne
sert que si le futur est *droppé*, et rien ne le droppe — le `cancel` n'est
sélectionné qu'à `spawn.rs:345`, c'est-à-dire **après** la sonde. Une sonde
pendante survit donc aussi à l'arrêt gracieux.

**Le silence ressemble à de la santé, et c'est le pire.** `manager_cadence_start`
(263) et `emit_delivery_resolved` (265) sont émis **avant** la sonde. Le journal
dit donc « cadence démarrée » puis « route résolue vers X » puis plus rien — et
mika#2267 a déjà écrit que l'**absence** de `manager_delivery_resolved` signifie
un binaire ancien. Sa **présence** suivie de silence est exactement la forme qui
ressemble le plus à un système sain. Seul le plancher heartbeat (6 h) rendrait le
silence anormal, et rien ne le nomme.

#### R2 — L'hypothèse la plus tentante sur AC3 est réfutée, et il faut le dire

Le ticket écrit : *« bare '401' in body could match other errors' innards »*.
La lecture cherchait un « innard » précis et plausible : la voie de livraison.
`HttpReportDeliverer::deliver` produit sur non-2xx
`delivery failed: 401 Unauthorized — <corps>` (`cadence.rs:338-341`). Cette
chaîne, classée par `classify_cycle_error`, donnerait `Unauthorized` et
alimenterait l'alarme **GitHub** pour un problème de
`MIKA_MANAGER_DELIVERY_TOKEN`, avec un hint envoyant l'opérateur lire
`MIKA_GITHUB_TOKEN` dans `/proc/.../environ`.

**Cette chaîne n'atteint pas le classificateur.** Le bras non-2xx retombe sur le
sink hors-ligne (`cadence.rs:537-570`), pose `delivered = true`, et ne propage
rien. Hypothèse **réfutée** ; ne pas construire dessus. Les échecs de livraison
ont d'ailleurs déjà leur propre surface typée (`DeliveryError` +
`classify_delivery_auth`, mika#1949).

#### R3 — Le vecteur réel est que le classificateur lit ses propres arguments

Ce qui atteint `classify_cycle_error(&format!("{e}"))` (`spawn.rs:420`) vient
pour l'essentiel de `Reader::read`, dont l'erreur est
`anyhow!("gh {} failed: {}", args.join(" "), stderr)` (`reader.rs:62`).

**La ligne de commande est donc dans le texte classé.** Or elle porte des
chiffres que l'opérateur choisit : `/repos/{owner}/{repo}/milestones/{N}`,
`--milestone {N}`, `--search milestone:{N}`, et le slug du dépôt.

Conséquence concrète : avec `MIKA_MANAGER_TARGET_MILESTONE=owner/repo#403`,
**toute** défaillance de cet appel — un 500, un échec DNS dont le libellé ne
tombe dans aucun motif `Network`, un échec de parsing JSON — est classée
`Forbidden`. `is_auth_failure` admet `Forbidden` (`spawn.rs:896-898`), donc
l'alarme de 30 minutes se déclenche avec la mauvaise classe et le mauvais hint.

**Borne d'honnêteté : aucune occurrence n'est mesurée.** Les milestones de `mika`
sont numérotés ~1-30, donc le déclencheur est étroit *aujourd'hui*. Le défaut est
**structurel** et le classificateur est générique sur n'importe quelle cible
posée par un opérateur. C'est un durcissement p3, pas la réparation d'un
incident.

#### R4 — La contre-pression sur AC3 est plus forte que l'AC ne le laisse voir

`classify_cycle_error` n'est pas un simple champ de journal : il alimente
`is_auth_failure` → `AuthFailureTracker::on_failure`, qui rend `None` pour toute
classe non-auth (`spawn.rs:935-937`). **Chaque motif retiré est une forme de 401
qui devient `Other`, donc que le tracker ignore, donc un aveuglement silencieux
restauré** — exactement ce que mika#2013 a fermé et mika#2063 élargi.

Un « regex-narrow » naïf appliqué à cette fonction est donc une régression
déguisée en durcissement. D'où l'invariant D7 et la seule formulation acceptable
du resserrement : **restreindre l'espace de recherche et ancrer les motifs
numériques, sans jamais retirer un motif non-numérique.**

---

## Product Contract

### Symptôme observable aujourd'hui

| # | Symptôme | Surface |
|---|---|---|
| 1 | Un `gh api` pendant au démarrage laisse la cadence sans premier tick, indéfiniment, et le journal montre « cadence démarrée » suivi de silence | aucune |
| 2 | Un empoisonnement de `MANAGER_SPAWN_GUARD` fait paniquer tout appel ultérieur à `spawn_manager_cycle_task` | panique |
| 3 | Une défaillance de cycle sur une cible dont le numéro est `401`/`403` est classée comme un échec d'auth et déclenche l'alarme GitHub | `manager_cycle_error auth_class=` |

### Périmètre

**Dans le périmètre :** AC1, AC2, AC3 — items 1 à 3 du ticket.

**Hors périmètre, sur décision opérateur explicite :** AC4 (rafraîchissement
proactif du token App en milieu de cycle). Le commentaire de samidarko du
2026-08-29 le sort de ce ticket : *« Le 4ᵉ item est le sujet entier de
mika#2013 »*, `p1-important`, groomé, avec un spawn en cours. mika#2013 a depuis
livré ce correctif (`refresh_cycle_token`, `spawn.rs:792-830`, re-résolution
avant chaque cycle). **Rien de ce plan ne touche cette voie.** Si quelqu'un
rouvre l'item, la réponse est mika#2013.

### Ce que ce travail n'achète pas

- **Aucune borne sur le corps du cycle.** `Reader::read` fait trois appels `gh`
  sans timeout ; un blocage y fige la boucle *en cours de cycle* et
  `interval.tick()` ne refire jamais. Même classe, blast radius différent,
  remède différent — voir D9.
- **Aucun événement nouveau ne mesure le symptôme 3 rétroactivement.** La
  distribution de `auth_class` avant/après le déploiement est le seul
  instrument, et elle enjambe un changement de vocabulaire (D3 ajoute une
  valeur).
- **Aucune migration, aucune variable d'environnement, aucun changement de
  `.env.example`.**

---

## Planning Contract

### D1 — Le budget est une propriété de `verify_gh_auth`, pas de son appelant

`tokio::time::timeout` enveloppe `runner.run(...)` **à l'intérieur** de
`verify_gh_auth`, pas le `verify_gh_auth(...).await` du site d'appel.

Même raisonnement que la garde 5d de mika#2290, qui prend `deployment` en
paramètre *« plutôt que de le laisser à un `if` côté appelant, pour que la
propriété soit celle de la fonction pure et porte son propre test »*. Il n'y a
qu'un appelant en production aujourd'hui (`spawn.rs:280`) et la fonction n'est
pas réexportée depuis `mod.rs` — mais elle est `pub` dans le module, et un futur
appelant hériterait de la borne au lieu de devoir s'en souvenir.

### D2 — 15 s, constante nommée. Divergence assumée avec le littéral de l'AC

AC1 dit `5s`. Le plan pose `GH_AUTH_PROBE_TIMEOUT = Duration::from_secs(15)`,
pour trois raisons.

1. **La borne supérieure ne coûte rien d'observable.** Le premier tick réel est
   à `poll_interval` (300 s par défaut) *après* la sonde, parce que
   `interval.tick().await` (`spawn.rs:342`) consomme délibérément le tir
   immédiat. 15 s contre 300 s de marge, c'est 5 % de retard sur le premier
   cycle dans le seul cas où la sonde est lente.
2. **La borne inférieure coûte, elle.** La sonde est *fail-open* (log-and-continue),
   donc un faux timeout ne bloque aucun cycle — mais il écrit un `error!`
   `manager_gh_auth_check_failed` sur la surface que mika#2013 existe pour rendre
   digne de confiance. Un réseau simplement lent produisant une ERREUR d'auth au
   démarrage entraîne l'opérateur à ignorer cette ligne. `gh` est un binaire Go
   à démarrer, plus une poignée de main TLS, plus un GET : un p99 de quelques
   secondes sur un hôte chargé n'est pas une anomalie.
3. **15 s est déjà le seul chiffre de timeout réseau du module.**
   `TOKEN_REFRESH_TIMEOUT` (`spawn.rs:773`), `HttpReportDeliverer`
   (`cadence.rs:289`) et `HttpAuthAlarmSink` (`spawn.rs:1002`) le posent tous
   trois. Un second chiffre différent est un nombre de plus à relire.

**Refusé : une variable d'environnement.** Précédent explicite dans ce fichier —
`AUTH_PERSISTENT_FAILURE_THRESHOLD` porte en toutes lettres *« Deliberately NOT
env-configurable in v1 (YAGNI) … a named constant suffices until an operator
expresses the need to tune it »*. Aucun opérateur n'a exprimé ce besoin. En
prime, une variable ici déclencherait
`mika2267_every_manager_env_const_is_declared_in_env_example`, donc un aller-retour
`.env.example` pour un réglage que personne ne demande.

**Si l'architecte préfère le littéral 5 s de l'AC, le changement est d'une
constante** et aucun autre élément du plan ne bouge.

### D3 — Une valeur `AuthClass::Timeout`, sur l'événement existant

Le timeout rend `Err(GhAuthError { auth_class: AuthClass::Timeout, … })`, et le
`match` du site d'appel (`spawn.rs:297-311`) gagne son bras de hint. L'événement
reste `manager_gh_auth_check_failed` avec `auth_class=timeout`.

**Refusé : réutiliser `AuthClass::Network`.** Son hint dit *« gh cannot reach
GitHub — check network/DNS/TLS »*, ce qui est faux d'un réseau seulement lent, et
cela fusionnerait deux populations dont les remèdes diffèrent (« le réseau est
mort » vs « la sonde est trop serrée ou l'hôte est chargé »).

**Refusé : un nom d'événement distinct**, à la manière de
`manager_token_refresh_timeout`. Ce voisin a eu besoin de son propre nom parce
qu'il n'a **aucun champ de classe** ; la sonde d'auth a un événement unique
*avec* un discriminateur de classe, et les opérateurs greppent déjà `auth_class=`.
Ajouter une valeur est dans le grain ; ajouter un second nom fragmenterait la
population « la sonde de démarrage n'est pas passée ».

**`Timeout` reste hors de `is_auth_failure`.** Un réseau lent n'est pas une porte
fermée. Le test de frontière existant
(`auth_alarm_never_fires_for_non_auth_classes`, `spawn.rs:2362`) est étendu à la
nouvelle variante — c'est lui qui pinne la décision.

**`as_str()` est un format de fil** (les opérateurs en font des `grep` et des
`GROUP BY`). La valeur est `"timeout"`, et les six valeurs sont épinglées par
test. Note : `AuthAlarmBody.auth_class` (`spawn.rs:973`) est un format de fil
*sortant*, sur l'endpoint d'escalade — il ne peut porter que `401`/`403` puisque
seul `is_auth_failure` y mène, donc **ajouter la variante ne change pas ce
fil-là**.

### D4 — AC2 : dissoudre l'empoisonnement plutôt que s'en relever

AC2 prescrit `.unwrap_or_else(|e| e.into_inner())` plus un `warn!`. Le plan pose
à la place un `AtomicBool` et un `swap(true, SeqCst)`, ce qui supprime le verrou
et donc la question. Trois raisons.

1. **Le `warn!` prescrit est injoignable dans le cas même qui le produit.** La
   section critique (`spawn.rs:238-251`) contient exactement trois choses : la
   lecture d'un `bool`, un `warn!`, l'écriture d'un `bool`. `as_display()` est un
   `format!` sur un `String` et un `u64` (`types.rs:46-48`) et ne peut pas
   paniquer. **Le seul site de panique réaliste est la macro de journalisation
   elle-même** — donc un chemin de relèvement qui journalise est peu fiable
   précisément quand il sert.
2. **Le blast radius en production est nul.** Il y a un seul appelant
   (`server/mod.rs:1471`), une fois par processus. Un verrou empoisonné
   n'affecterait aucun appel ultérieur puisqu'il n'y en a pas. L'exposition
   réelle est le binaire de test, où un statique empoisonné casse en cascade des
   tests sans rapport via `reset_spawn_guard_for_test` (3 sites d'appel :
   `spawn.rs:1630, 2084, 2218`).
3. **`swap(true)` est le compare-and-set que le verrou émulait.** La forme
   actuelle est lire-puis-écrire sous verrou ; un `swap` atomique dit la même
   chose en une opération, sans `.unwrap()` sur un statique partagé.

**Coût nommé :** le `warn!` que l'AC demande devient injoignable **et inutile**.
Un événement dont le régime attendu est « impossible » est pire que pas
d'événement : il suggère une population qui n'existe pas.

**Conséquence documentaire obligatoire.** Le docstring de
`MANAGER_SPAWN_GUARD` (`spawn.rs:50-68`) **argumente aujourd'hui en faveur de
`Mutex<bool>`** (*« `Mutex<bool>` (not `OnceLock`) is deliberate … a `Mutex`
allows the test suite to reset the guard »*). Le laisser tel quel poserait une
prose qui contredit son propre code. Il est réécrit : l'`AtomicBool` garde la
même résettabilité en test, sans verrou ni empoisonnement.

### D5 — AC3, première moitié : restreindre l'espace de recherche en **typant** l'erreur

`reader.rs` gagne un `GhCommandError` (`thiserror`, déjà dans les dépendances du
crate) portant `args` et `stderr` **séparément**, dont le `Display` rend
**exactement** la chaîne d'aujourd'hui — `gh {args} failed: {stderr}` — pour que
rien en aval ne bouge (journaux, `stderr_head`, tests existants).
`ProcessGhRunner::run` rend `Err(anyhow::Error::new(GhCommandError { … }))`.
`classify_cycle_error` fait un `downcast_ref` et classe sur `.stderr` seul.

**Précédent maison, dans ce module même :** `DeliveryError` +
`err.downcast_ref::<DeliveryError>()` (`cadence.rs:256`). Et la règle de
mika#2179 : *« les classes d'erreur viennent de la variante, jamais d'une
correspondance de sous-chaîne sur le message rendu »*.

**Refusé : couper la chaîne sur `" failed: "` dans le classificateur.** C'est une
heuristique de sous-chaîne pour réparer une heuristique de sous-chaîne, et elle
casse le jour où un message contient ce séparateur.

**Repli explicite, fail-open vers le comportement d'aujourd'hui :** quand le
`downcast` échoue (erreur de parsing JSON, erreur d'I/O du sink, mock de test),
le classificateur lit `format!("{e}")` comme aujourd'hui. Volontairement `{e}` et
non `{e:#}` : élargir le texte vu ajouterait des faux positifs, ce qui est
l'inverse du but.

`GhCommandError` est `pub(crate)` : il voyage boxé dans un `anyhow::Error`, donc
aucune signature publique ne l'expose. Le passer `pub` plus tard est additif.

### D6 — AC3, seconde moitié : ancrer les motifs numériques, préserver les autres

Un helper pur `has_http_status(lower: &str, code: &str) -> bool` remplace les
`contains("401")` / `contains("403")` / `contains("404")`. Il accepte le code
quand il a des **frontières non-chiffres** *et* qu'un marqueur de statut
(`http`, `status`, `code`) figure dans une courte fenêtre qui le précède.

| chaîne | verdict | pourquoi |
|---|---|---|
| `http 401` | ✔ | marqueur `http` |
| `bad credentials (http 401)` | ✔ | idem |
| `http/2 401 unauthorized` | ✔ | idem |
| `"status":"401"` | ✔ | marqueur `status` |
| `/milestones/401` | ✘ | aucun marqueur en amont |
| `--milestone 403` | ✘ | idem |
| `milestone:401` | ✘ | idem |
| `4011`, `1401` | ✘ | frontière chiffre |

**Refusé : un simple `contains("http 401")`.** Il couvrirait toutes les formes
déjà présentes dans la suite de tests, donc paraîtrait suffisant — et manquerait
la forme de corps JSON `"status":"401"` que GitHub émet. Un resserrement qui
passe les tests existants en perdant un vrai positif est exactement le piège de
D7.

**Refusé : toucher aux motifs non-numériques.** `unauthorized`,
`bad credentials`, `gh auth login`, `authentication token not found` restent tels
quels. Les deux derniers sont les ajouts de mika#2013 pour la forme *token
absent*, qui **ne porte aucun statut HTTP** puisque `gh` n'atteint jamais l'API
et imprime son propre texte d'onboarding. Les retirer rendrait `Other` un
manager sans token — l'aveuglement exact que mika#2013 a fermé.

`classify_milestone_probe_error` subit le même traitement pour `404`, en gardant
son `not found` textuel et son ordre de délégation (mika#2013 a déjà corrigé cet
ordre : déléguer d'abord, discriminer 404 ensuite).

### D7 — L'invariant : ce changement ne retire que des faux positifs

**Formulation :** pour toute chaîne d'erreur qui classait `Unauthorized`,
`Forbidden` ou `MilestoneNotFound` avant ce ticket **et qui porte réellement ce
signal**, le classificateur rend la même classe après.

**Pourquoi c'est l'invariant central et pas une précaution :** la classe alimente
`is_auth_failure` → l'alarme d'auth persistante. Un vrai 401 rétrogradé en
`Other` est ignoré par le tracker et le manager redevient aveugle ~14 h, la durée
du silence de l'incident fondateur de mika#2013.

**Épinglé par un corpus figé** : toutes les chaînes 401/403/404 déjà présentes
dans la suite (`spawn.rs:1861-1920, 2604-2645`) plus les quatre formes
mika#2013, assertées inchangées. Ce corpus ne se « rafraîchit » pas : il décrit
des formes mesurées de `gh`.

### D8 — Une garde structurelle sur le typage, parce qu'aucune assertion ne le protège

Revenir à `anyhow!("gh {} failed: {}", …)` dans `ProcessGhRunner::run` **ne
casserait aucun test** : le repli chaîne de D5 garde toute la suite au vert
pendant que les arguments reviennent silencieusement dans le texte classé. C'est
exactement la classe qu'un test comportemental ne peut pas voir — la régression
ne rend aucune décision fausse, elle réélargit l'espace de recherche sans bruit.

Donc : un scan de source sur `reader.rs` refusant le littéral `anyhow!("gh `,
**allowlist livrée vide** (il n'y a rien à exempter, donc pas de case où déposer
le prochain écart — doctrine mika#2323). Plus un test d'**identité octet par
octet** du `Display` de `GhCommandError` avec le format hérité, pour que le
resserrement ne puisse pas non plus être défait par un changement de `Display`.

### D9 — Hors périmètre, nommé

- **AC4** — mika#2013, livré. Voir « Périmètre » ci-dessus.
- **Les appels `gh` du corps de cycle.** Même absence de borne, blast radius
  supérieur (la boucle se fige en régime établi, pas au démarrage). Le remède
  n'est pas le même : un budget par appel doit composer avec `poll_interval` et
  avec l'annulation — `tokio::time::interval` est en `MissedTickBehavior::Burst`
  par défaut, donc un cycle plus long que le poll refire immédiatement au retour.
  **Ticket de suivi**, à ouvrir avec cette raison.
- **Un timeout global dans `ProcessGhRunner::run`**, qui fermerait les deux d'un
  coup. Refusé ici : le bon budget d'un `gh api /milestones/N` n'est pas celui
  d'un `gh pr list --limit 100 --search`, et une constante unique serait soit
  trop serrée pour les listes soit trop lâche pour la sonde. De plus le runner
  sert aussi `mika milestone read|assess|report` en CLI, dont le profil de
  latence acceptable est celui d'un opérateur qui attend. Décision à part,
  rattachée au ticket de suivi ci-dessus.

---

## Implementation Units

### U1 — `reader.rs` : l'erreur `gh` devient typée (support de D5)

- Ajouter `GhCommandError { args: String, stderr: String }`, `pub(crate)`,
  `#[derive(Debug, thiserror::Error)]`, `#[error("gh {args} failed: {stderr}")]`.
- `ProcessGhRunner::run` : remplacer
  `Err(anyhow!("gh {} failed: {}", args.join(" "), stderr))` par
  `Err(anyhow::Error::new(GhCommandError { args: args.join(" "), stderr: stderr.into_owned() }))`.
- Aucun autre site de `reader.rs` ne bouge ; `Reader::read` continue de propager
  par `?`.

### U2 — `spawn.rs` : le classificateur (AC3, D5 + D6)

- Ajouter `has_http_status(lower: &str, code: &str) -> bool` (fonction pure,
  frontières non-chiffres + marqueur de statut en amont).
- Renommer le corps actuel en `classify_cycle_error_text(text: &str) -> AuthClass`
  et y remplacer les trois `contains` numériques par `has_http_status`. Les
  motifs non-numériques sont inchangés.
- Nouvelle `classify_cycle_error(err: &anyhow::Error) -> AuthClass` :
  `downcast_ref::<GhCommandError>()` ⇒ classer sur `.stderr` ; sinon classer sur
  `format!("{err}")`.
- Même dédoublement pour `classify_milestone_probe_error`
  (`…_text` + variante `&anyhow::Error`), en conservant l'ordre de délégation.
- Mettre à jour les deux sites d'appel : `spawn.rs:420`
  (`classify_cycle_error(&e)`) et `spawn.rs:696`
  (`classify_milestone_probe_error(&e)` — `e` est déjà en main).
- Mettre à jour les docstrings des deux classificateurs : dire ce qui est classé
  (le `stderr`, pas la ligne de commande) et pourquoi les motifs non-numériques
  ne bougent pas (D7).

### U3 — `spawn.rs` : la sonde est bornée (AC1, D1 + D2 + D3)

- `const GH_AUTH_PROBE_TIMEOUT: Duration = Duration::from_secs(15);` avec le
  raisonnement de D2 en docstring (les deux bornes, et la marge de 300 s).
- `AuthClass::Timeout` + bras `as_str()` ⇒ `"timeout"` + docstring de variante.
- Dans `verify_gh_auth` : envelopper `runner.run(&["api", &path])` dans
  `tokio::time::timeout(GH_AUTH_PROBE_TIMEOUT, …)`. Sur `Err(Elapsed)`, rendre
  `GhAuthError { auth_class: Timeout, stderr_head: format!("probe timed out after {}s", …), exit_code: -1 }`.
- Ajouter le bras de hint `AuthClass::Timeout` dans le `match` de
  `spawn.rs:297-311` (le `match` est exhaustif : le compilateur l'exige).

### U4 — `spawn.rs` : la garde sans verrou (AC2, D4)

- `static MANAGER_SPAWN_GUARD: AtomicBool = AtomicBool::new(false);`
- Section critique ⇒ `if MANAGER_SPAWN_GUARD.swap(true, Ordering::SeqCst) { warn!(…); return None; }` —
  le `warn!` de double-spawn est conservé tel quel.
- `reset_spawn_guard_for_test()` ⇒ `MANAGER_SPAWN_GUARD.store(false, Ordering::SeqCst);`
- `use std::sync::{Arc, Mutex}` (`spawn.rs:44`) ⇒ `use std::sync::Arc;` +
  `use std::sync::atomic::{AtomicBool, Ordering};` (`Mutex` reste importé
  séparément dans `mod tests`, `spawn.rs:1254`).
- Réécrire le docstring `spawn.rs:50-68` (D4, dernier paragraphe) en préservant
  la discrimination un-processus-vs-deux-processus, qui est le contenu utile.

### U5 — Tests et gardes

Voir Verification Contract. Tous les tests vivent dans `mod tests` de
`spawn.rs`, sauf ceux de U1 qui vivent dans `reader.rs`.

---

## Verification Contract

### AC3 — le classificateur

| test | assertion | pourquoi il est nécessaire |
|---|---|---|
| `mika1975_has_http_status_units` | les 8 lignes du tableau D6 | le prédicat pur, isolé de tout le reste |
| `mika1975_typed_error_classifies_on_stderr_only` | `GhCommandError { args: "api /repos/o/r/milestones/401", stderr: "HTTP 500: Internal Server Error" }` ⇒ `Other` | **contrôle négatif du typage** |
| `mika1975_untyped_string_still_narrows_the_numeric_patterns` | la même chose en chaîne plate `"gh api /repos/o/r/milestones/401 failed: HTTP 500: Internal Server Error"` ⇒ `Other` | **contrôle négatif du resserrement de motif** |
| `mika1975_the_classification_of_every_measured_auth_shape_is_unchanged` | corpus figé (D7) ⇒ classes identiques | **l'invariant qui protège l'alarme** |
| `mika1975_the_gh_command_error_display_is_byte_identical` | `format!("{err}")` == format hérité | protège `stderr_head` et les journaux |
| `mika1975_the_gh_runner_error_stays_typed` | scan de source sur `reader.rs`, allowlist vide | D8 |

**Les deux contrôles négatifs sont tous les deux requis, et la raison est à
écrire dans les tests.** Le chemin typé seul satisferait le contrôle négatif
tout en laissant le resserrement de motif non testé (les arguments étant déjà
partis) ; le chemin non typé seul laisserait le typage non testé. Chacun est
vérifié rouge en neutralisant **sa** moitié, pas les deux à la fois — leçon
mika#2277.

### AC1 — la sonde

| test | assertion |
|---|---|
| `mika1975_the_auth_probe_is_bounded` | `#[tokio::test(start_paused = true)]`, mock dormant 60 s ⇒ `Err(auth_class == Timeout)` |
| `mika1975_a_probe_just_under_the_budget_succeeds` | mock dormant 14 s ⇒ `Ok(())` — contrôle négatif : sans lui, « la sonde est bornée » ne se distingue pas de « la sonde échoue toujours » |
| `auth_alarm_never_fires_for_non_auth_classes` (étendu) | `Timeout` ajouté à la boucle ⇒ jamais d'alarme |
| `mika1975_auth_class_as_str_is_a_wire_format` | les six valeurs épinglées |

Le mock dormant est un nouveau `SleepingGhRunner` dans `mod tests` ; l'horloge
virtuelle rend les deux tests instantanés et déterministes.

### AC2 — la garde

- `spawn_manager_cycle_task_second_call_rejected` (`spawn.rs:2083`) passe
  inchangé : c'est le test de comportement de la garde, et son comportement ne
  change pas.
- **Aucun test d'empoisonnement n'est ajouté, et c'est délibéré** : l'état est
  injoignable par construction après D4. Asserter « ça ne s'empoisonne pas » sur
  un type qui n'a pas d'empoisonnement est vide de sens.

### Portée non touchée, à vérifier verte

- `no_dispatch_scaffolding_in_milestone_manager` — aucun `FORBIDDEN_TOKENS`
  introduit (ni `gh api -X …`, ni `"PATCH"`, ni `dispatcher_source`).
- `mika2267_every_manager_env_const_is_declared_in_env_example` — **aucune
  variable d'environnement ajoutée** (D2), donc aucun changement `.env.example`.
- Aucune migration de schéma, aucune ligne `audit_events` nouvelle.

### Commandes

```bash
cargo test -p mika-agent milestone_manager
cargo test -p mika-agent            # la suite complète : le classificateur est module-local, mais AuthClass a gagné une variante
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

### Sondes post-déploiement, et leurs haltes

```bash
# 1 — la sonde de démarrage
grep manager_gh_auth_check_failed "$MIKA_SPIRIT_LOG_FILE" | jq '{auth_class, stderr_head, hint}'
```
**Régime attendu : zéro ligne.** Des lignes `auth_class == "timeout"` signifient
que la sonde est coupée. **Halte : ne pas relever le budget par réflexe** — la
première question est si `gh` est lent sur cet hôte, et un budget relevé
rallonge le blocage que la borne existe pour borner.

```bash
# 2 — la distribution des classes de cycle
grep manager_cycle_error "$MIKA_SPIRIT_LOG_FILE" | jq -r .auth_class | sort | uniq -c
```
Après le correctif, `401`/`403` ne doivent plus apparaître sur des cycles dont
l'erreur est un 500 ou un échec de parsing. **Halte : si `403` domine encore sur
une cible numérotée 403, le chemin typé n'est pas en vigueur** — établir que le
binaire qui tourne est celui qui a été construit (classe mika#2340) **avant** de
toucher aux motifs.

```bash
# 3 — l'invariant D7, en production
grep manager_auth_persistent_failure "$MIKA_SPIRIT_LOG_FILE"
```
Cette population **ne doit pas passer de non-vide à vide** du fait de ce
changement. Si elle le fait, un vrai positif a été rétrogradé : l'invariant D7
échoue en production, et le remède est de **restaurer le motif**, pas d'élargir
la fenêtre de l'alarme.

**Le silence ne prouve rien pour les sondes 1 et 3** : la sonde de démarrage ne
tire qu'à un redémarrage, et l'alarme ne tire qu'après 30 minutes d'échec
continu. Vérifier qu'un redémarrage a bien eu lieu avant de conclure.

---

## Definition of Done

- [ ] `verify_gh_auth` borne son appel `gh` par `GH_AUTH_PROBE_TIMEOUT` (15 s,
      constante nommée, non env-configurable) et rend
      `AuthClass::Timeout` sur dépassement.
- [ ] Le site d'appel (`spawn.rs:297-311`) porte un bras de hint pour `Timeout`.
- [ ] `AuthClass::Timeout` est hors de `is_auth_failure`, asserté par le test de
      frontière existant étendu.
- [ ] `as_str()` rend `"timeout"` et les six valeurs sont épinglées comme format
      de fil.
- [ ] `MANAGER_SPAWN_GUARD` est un `AtomicBool` ; plus aucun `.unwrap()` sur un
      statique dans ce fichier ; son docstring ne contredit plus son code.
- [ ] `reset_spawn_guard_for_test` et
      `spawn_manager_cycle_task_second_call_rejected` passent inchangés en
      comportement.
- [ ] `ProcessGhRunner::run` rend un `GhCommandError` typé dont le `Display` est
      identique octet par octet au format hérité.
- [ ] `classify_cycle_error` et `classify_milestone_probe_error` prennent
      `&anyhow::Error`, classent sur le `stderr` typé, et retombent sur la chaîne
      complète en fail-open.
- [ ] Les motifs numériques passent par `has_http_status` ; **aucun motif
      non-numérique n'est retiré**.
- [ ] Les deux contrôles négatifs (typé, non typé) existent et ont été vérifiés
      rouges en neutralisant chacun sa moitié.
- [ ] Le corpus figé de D7 asserte l'invariance de classification de toutes les
      formes d'auth mesurées.
- [ ] Le scan de source de D8 refuse le retour de `anyhow!("gh ` dans
      `reader.rs`, allowlist vide.
- [ ] AC4 n'est pas implémenté ; le plan et le corps de PR citent mika#2013.
- [ ] Le ticket de suivi sur les appels `gh` non bornés du corps de cycle est
      nommé dans le corps de PR (`Tracked in:`).
- [ ] `cargo test -p mika-agent`, `cargo clippy -- -D warnings`, `cargo fmt --check`
      passent.
- [ ] Aucune migration, aucune variable d'environnement, aucun changement
      `.env.example`.

---

## Acceptance criteria

Transcrits verbatim de `senara-solutions/mika#1975` § AC.

- **AC1** : `verify_gh_auth` wrapped in `tokio::time::timeout(5s, ...)` avec log si timeout
- **AC2** : `MANAGER_SPAWN_GUARD` handles `PoisonError` via `.unwrap_or_else(|e| e.into_inner())` avec warn log
- **AC3** : `classify_cycle_error` regex-narrow ou HTTP status parse (rejette bare substring matches)
- **AC4** : app-token proactive refresh check (call token freshness API OR refresh N minutes before expiry) — decisionale via mika-common auth chain

### Divergences assumées, et leur arbitrage

| AC | lettre | livré | raison |
|---|---|---|---|
| AC1 | `5s` | `15s` | D2 — trois raisons ; retour au littéral = une constante |
| AC1 | « log si timeout » | `manager_gh_auth_check_failed auth_class=timeout` + hint | D3 — une valeur sur l'événement existant plutôt qu'un nom de plus |
| AC2 | relèvement de `PoisonError` + `warn!` | `AtomicBool` — l'empoisonnement est dissous | D4 — le `warn!` prescrit est injoignable dans le cas qui le produit |
| AC3 | « regex-narrow » | espace de recherche typé **plus** motifs numériques ancrés, motifs non-numériques préservés | D6 + D7 — un resserrement naïf retirerait des vrais positifs et rouvrirait mika#2013 |
| AC4 | — | **non livré** | Commentaire opérateur du 2026-08-29 : sujet entier de mika#2013, depuis livré |

---

## Sources

- `senara-solutions/mika#1975` — corps + les deux commentaires opérateur
  (réduction de périmètre du 2026-08-29, dé-parquage du 2026-09-20).
- `senara-solutions/mika#1968` PR#1973 — parent ; origine des items A5/A6/A7 et
  C1/C2.
- `senara-solutions/mika#1974` — sonde milestone-scoped ; origine de
  `classify_milestone_probe_error` et de `AuthClass::MilestoneNotFound`.
- `senara-solutions/mika#2013` — re-résolution du token par cycle (AC4) et
  alarme d'auth persistante ; source des motifs non-numériques que D6 préserve.
  `docs/solutions/logic-errors/2013-token-resolved-once-at-spawn-freezes-a-renewable-credential.md`.
- `senara-solutions/mika#2063` — `Forbidden` rejoint `is_auth_failure` ; ce qui
  rend le resserrement de `403` aussi sensible que celui de `401`.
- `senara-solutions/mika#1949` — `DeliveryError`, `AuthBoundaryLedger` ; le
  précédent de typage d'erreur dans ce module (`cadence.rs:256`).
- `senara-solutions/mika#2179` — *les classes d'erreur viennent de la variante,
  jamais d'une sous-chaîne sur le message rendu*.
- `senara-solutions/mika#2290` — le précédent « le prédicat prend son paramètre
  plutôt qu'un `if` côté appelant » (D1) et le `match` exhaustif sans bras `_`.
- `senara-solutions/mika#2323` — allowlist de garde structurelle livrée vide.
- `senara-solutions/mika#2267` — `manager_delivery_resolved` ; pourquoi la
  présence de cette ligne suivie de silence est la forme qui ressemble le plus à
  la santé (R1).
- `senara-solutions/mika#2205` — un scan silencieusement inerte se lit comme un
  scan oisif.
- Code lu : `crates/mika-agent/src/milestone_manager/{spawn.rs,reader.rs,cadence.rs,types.rs,no_dispatch_test.rs}`,
  `crates/mika-agent/src/server/mod.rs:1444-1495`.
