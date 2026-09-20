# mika#2049 — Le repli d'egress devient fail-closed

**Ticket :** [`senara-solutions/mika#2049`](https://github.com/senara-solutions/mika/issues/2049)
**Type :** fix (posture de sécurité)
**Décision opérateur :** option 1, **fail-closed**, tranchée par Vincent le 2026-09-20 après bearing de Prime. L'option 2 (variable d'échappement) est **écartée par écrit** : « elle recrée le fail-open sous un autre nom, et un WARN sous charge n'est lu par personne. »

---

## 1. Ce qui est mesuré, et les deux corrections que la lecture du code apporte au ticket

### 1.1 Le défaut, à la ligne près

`_ensure_pilot_egress_proxy` (`skills/bundled/_shared/dispatch-lib.sh:491`) rend `1` sur trois
causes distinctes, et son unique appelant décisionnel
(`_run_pilot_sandboxed`, `dispatch-lib.sh:973`) lit ce `1` comme « lance en Phase 2a » :

| Cause | Ligne | Message actuel |
|---|---|---|
| binaire absent ou non exécutable | `493` | `mika-pilot-egress-proxy not found at … — Phase 2b network cut disabled (falling back to fs-only)` |
| bind non obtenu en 3 s | `531` | `pilot_egress_guard.unreachable … failed to bind … within 3s — falling back to fs-only` |
| *(implicite)* socket vivant mais non-servant | — | aucun — `_pilot_egress_sock_connectable` rend `0`, le lanceur dit « alive » |

Le ticket cite le commentaire des lignes 200-203 ; ce commentaire est aujourd'hui à
`dispatch-lib.sh:488-490` (le fichier a bougé). La justification écrite y est toujours celle
de la fenêtre de déploiement de #1894, close depuis.

### 1.2 Correction 1 — `pilot_egress_guard.unreachable` n'est lu par personne

Mesure : `grep -rn "pilot_egress_guard" . --exclude-dir=.git --exclude-dir=target` rend
**une seule ligne**, le site d'écriture lui-même. Aucun consommateur, aucun test, aucune
documentation. Le ticket dit que mika#2041 a rendu l'état « visible » ; il l'a rendu **émis**.
C'est exactement la classe que mika#2131 a dû nommer : *une observabilité qui n'atteint aucun
lecteur collecté ne distingue rien.* Le préalable 1 de l'opérateur (« escalade sur un canal
réellement lu, pas seulement un événement émis ») porte donc sur un signal qui n'a, à ce jour,
**aucun lecteur du tout** — pas même un grep opérateur documenté.

### 1.3 Correction 2 — le fail-closed casse le test négatif de l'opérateur s'il s'arrête au dernier moment

Le test négatif demandé est : *« proxy relancé ⇒ le dispatch reprend **sans intervention sur les
tickets** »*. Un refus posé uniquement au fond de `_run_pilot_sandboxed` ne le satisfait pas, et
l'arithmétique le dit :

1. le ticket promu `ready` est dispatché, le sandbox refuse, la tâche rend un `RESULT` de refus ;
2. le ticket **garde `ready`** et n'est pas dispatché ;
3. `auto_pull` Phase 2 le voit `ready` depuis > `MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS`
   (900 s) et le re-drive — `redrive_count` passe à 1 ;
4. à `MIKA_AUTO_PULL_MAX_REDRIVES` (défaut **3**), Phase 2 **abandonne** le ticket : elle pose
   `operator-review`, retire `ready`, et commente (mika#2020).

**Une panne de proxy d'environ 45 à 60 minutes parque donc chaque ticket `ready` derrière un
label opérateur**, et la reprise exige un geste par ticket — très précisément ce que l'AC exclut.
Le fail-closed ne peut pas être livré sans traiter cette conséquence ; c'est le quatrième
livrable du lot, et il n'est pas dans la liste de l'opérateur parce qu'il est dans son test.

### 1.4 Ce que la mesure ne dit pas, et qui n'est donc pas supposé

La cause de la casse du 15/09 n'est **pas** établie par ce plan. Mesure prise sur `gentux` le
2026-09-20 : `/tmp/mika-pilot-egress.sock` porte une mtime du **15 septembre 06:41** et le relais
sert normalement depuis — c'est-à-dire que le fail-closed, déployé aujourd'hui, n'arrêterait rien.
Le préalable 3 est donc livré comme **un geste de remise en marche outillé et testé plus un
runbook**, jamais comme le correctif d'une cause non mesurée. Un correctif de cause dont on ne
tient pas la cause est une réparation qu'on ne peut pas vérifier.

---

## 2. Requirements

**R1.** Egress indisponible ⇒ **aucun pilote ne démarre**. Le refus est inconditionnel : aucune
variable d'environnement ne le lève (l'option 2 est écartée par décision opérateur).

**R2.** Le refus porte un **motif distinct par cause** et **le geste de remise en marche**. Les
causes du tableau §1.1 appellent des remèdes différents (déployer le binaire / relancer le relais)
et doivent rester comptables séparément — précédent : `below_threshold` vs
`no_ready_label_event` (mika#2131).

**R3.** Le refus déclenche une **escalade sur un canal réellement lu**, par un chemin
**déterministe** — pas un tour LLM, pas une consigne de prompt
(`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`).

**R4.** L'escalade est **dédupliquée par épisode de panne** (pas une alerte par dispatch) et
**la reprise est annoncée** : un rail qui repart sans le dire laisse l'opérateur devant un silence
qu'il ne peut pas distinguer d'une panne persistante.

**R5.** À la reprise du relais, **la boucle repart sans geste sur les tickets** : aucun ticket ne
doit avoir été parqué du fait de la panne.

**R6.** Le relais d'egress a un **geste de remise en marche outillé, testé**, et un **runbook**.

**R7.** Aucune régression de contenance : les quatre invariants déjà tenus par tests
(`sandbox_git_usable`, `pilot-github-token-not-in-sandbox`, `sandbox_log_dir_bound`,
`sandbox_no_secret_in_argv`) restent verts — voir §5.2, ils **dépendent du fail-open** aujourd'hui.

---

## 3. Design

### 3.1 Trois gardes, et une seule d'entre elles protège

C'est la propriété centrale du lot, et la confondre affaiblirait la protection :

| | Où | Rôle | Lit un état persistant ? | Fail-* |
|---|---|---|---|---|
| **C** | `dispatch-lib::_run_pilot_sandboxed` | **la protection** — refuse le lancement | **non**, sonde à chaque fois | fail-**closed** |
| **B** | `auto_pull` Phase 2 (Rust) | économie — ne consomme pas de budget de re-drive | oui (stamp) | fail-**open** |
| **A** | `ready_label_handler` (Rust) | économie — ne crée pas de tâche ni de différé | oui (stamp) | fail-**open** |

**A et B sont des optimisations de confort opérateur, faillibles et fail-open ; C est la
protection, inconditionnelle, et ne lit aucun état persistant.** Un futur lecteur tenté de durcir
A ou B au motif qu'elles sont fail-open doit savoir que la sûreté ne repose pas sur elles : elle
repose sur C, qui sonde le socket à chaque dispatch. Inversement, quiconque affaiblirait C en
lui faisant lire le stamp de A/B transformerait la protection en cache, et un cache périmé est
précisément un fail-open avec une étape de plus.

### 3.2 Garde C — le refus (shell)

**Le lanceur ne décide plus, il rapporte.** `_ensure_pilot_egress_proxy` conserve son code de
retour (le canary `scripts/canary-pilot-containment:90` en dépend) et pose en plus un motif dans
une variable non-`local`, sur le modèle exact de `_PILOT_GITDIR_BIND_ABORT` (`dispatch-lib.sh:710`) :

```
_PILOT_EGRESS_ABORT="<motif structuré>"     # vide = le relais sert
```

Motifs (vocabulaire fermé, un par cause de §1.1) :

- `egress_binary_missing` — le binaire n'est pas à `$_PILOT_EGRESS_PROXY_BIN` ;
- `egress_bind_timeout` — lancé, pas de bind en 3 s.

**Le site d'appel refuse**, dans le `else` de `dispatch-lib.sh:973`, en réutilisant le canal
mika#2141 déjà câblé de bout en bout : `_PILOT_SANDBOX_REFUSAL=<texte> ; return 78`. Ce choix
livre **le préalable 2 presque intégralement et gratuitement** — `_run_claude_pilot` classe déjà
le code 78 en `CONTAINMENT REFUSAL (exit 78) — the pilot was never launched`
(`dispatch-lib.sh:2654`), texte qui dit explicitement que ce n'est ni une dérive du pilote ni un
échec de pipeline. Il ne reste qu'à enrichir le motif du **geste de remise en marche** (R2).

**Placement : avant `_stage_pilot_gh_token`, donc avant DEUX effets de bord et non un.** La
séquence réelle est `_stage_pilot_gh_token` (`:970`) puis `_ensure_pilot_helper || true` (`:971`)
puis la décision (`:973`) :

- `_stage_pilot_gh_token` (`dispatch-lib.sh:605`, `umask 077`) rafraîchit un credential GitHub
  hôte. Ce n'est pas une fuite nouvelle — le fichier existe déjà entre deux dispatches — mais
  c'est un credential rafraîchi pour un lancement qui n'aura pas lieu ;
- `_ensure_pilot_helper` **lance un daemon**, et le plan initial l'omettait. Un refus posé après
  lui laisse un helper démarré derrière un dispatch refusé, à chaque tentative d'une panne.

Le précédent maison tranche dans le même sens : la porte 2c de mika#2279 est placée « avant
l'étape 3, donc sans résolution de token ». Le refus remonte donc **avant les deux**.

Une contrainte d'ordre à ne pas casser en le déplaçant : le commentaire mika#2056 à `:967-969`
exige que le token soit staged **avant** le helper, « so the mitmdump github addon has a fresh
credential to inject on its very first request ». Remonter le refus au-dessus du couple préserve
cet ordre intact ; l'insérer *entre* les deux le romprait.

**Le stamp.** Sur refus, la garde C écrit `~/.mika/state/pilot-egress-down` ; sur succès, elle le
retire. Contenu : une ligne `<RFC3339-UTC> <motif>`. Contrairement à `auto-pull-stop` (mika#2329,
dont le contenu n'est délibérément jamais lu), **ce contenu est lu** — A et B ont besoin de la
fraîcheur, et la §3.4 explique pourquoi une existence nue produirait un blocage permanent.

### 3.3 Escalade — `mika notify`, dédupliquée (R3, R4)

Le canal est **`mika notify --channel telegram --severity escalate --text "…"`**
(`crates/mika-cli/src/commands/notify.rs`). Trois propriétés en font le bon choix, et aucune
alternative examinée ne les réunit :

1. **déterministe** — aucun tour LLM, donc aucune enforcement par prompt ;
2. **réellement lu** — livraison Telegram via le gateway, le canal où l'opérateur est ;
3. **fail-soft sans perdre la trace** — la notification est **d'abord** écrite en base (session
   `00000000-0000-0000-0000-700000710717`) puis envoyée ; un gateway mort laisse un avertissement
   sur stderr et la ligne en base, lisible par `mika status` et le dashboard.

Écartés, avec leur raison : un commentaire GitHub sur le ticket (pendant une panne il y en aurait
un par ticket, et il n'est lu que par qui regarde le ticket) ; `mika ask --agent mika` (il faudrait
que le modèle décide d'émettre — enforcement par prompt sur substrat de boucle, refusé) ; un
`audit_events` seul (c'est la définition même de « seulement un événement émis » que le préalable 1
récuse).

**Déduplication par épisode** : l'alerte n'est émise que si le stamp était **absent** avant ce
refus. Une panne longue produit donc une alerte, pas une par dispatch.

**Reprise annoncée** : quand la garde C obtient le relais alors que le stamp était **présent**,
elle émet une seconde notification (`severity: info`) et retire le stamp. Modèle explicite :
`auto_pull_stop_armed` / `auto_pull_stop_lifted` (mika#2329) — une transition, jamais un état
répété.

**L'alerte ne conditionne jamais le refus.** `mika notify` est appelé en `|| true` : le refus est
la protection, l'alerte est l'information, et une alerte qui échoue ne doit pas rendre le
lancement au pilote. C'est l'ordre inverse de celui qu'un `set -e` mal placé produirait.

#### 3.3.1 Le canal a deux préconditions, et son échec est muet — c'est la moitié dure de l'AC1

Mesuré dans `crates/mika-cli/src/commands/notify.rs`, et ce sont les trois faits qui décident la
forme du livrable :

1. **`mika notify` rend `Ok(())` même quand Telegram échoue.** Lignes 73-90 : l'échec de
   `send_via_gateway` est attrapé, écrit en `eprintln!` (« ⚠ Telegram delivery failed »), puis la
   fonction rend `Ok`. Seul un échec d'écriture en base rend non-zéro. Le `|| true` de la §3.3
   garde donc son utilité **pour ce cas-là uniquement** (base illisible) et n'en a aucune pour
   l'échec de livraison, qui ne remonte pas. **Conséquence portante : l'appelant shell ne peut
   structurellement pas savoir si l'alerte a atteint quelqu'un.**
2. **La livraison exige un `chat_id` en base**, lu par `get_customer_config("chat_id")` sur
   l'agent **`mika`** (`NOTIFICATIONS_AGENT`, constante du module) — jamais sur mika-dev, qui est
   l'agent du dispatch. Absent ⇒ `bail!` (« no Telegram pairing yet ») ⇒ avalé par le point 1.
3. **Elle exige `MIKA_INTERNAL_TOKEN`**, lu depuis `ctx.settings` et non depuis l'environnement du
   process — donc résolu via `~/.mika/.env`, ce qui le rend **insensible au `scrub_mika_env_vars`**
   qui retire tous les `MIKA_*` des enfants de dispatch. Cette moitié-là est saine ; c'est la
   précondition 2 qui est fragile.

**Ce que ça change pour l'AC1.** Le préalable de l'opérateur distingue « une escalade sur un canal
réellement lu » de « seulement un événement émis ». Appeler `mika notify` produit un **appel
émis** ; si le `chat_id` n'est pas appairé, on obtient exactement le défaut que l'AC1 récuse, en
pire — silencieux de bout en bout, l'échec étant avalé deux fois (par le `Ok(())` et par le
`|| true`). **Une alerte qu'on ne peut pas vérifier n'est pas une escalade, c'est un espoir**
(formulation mika#2293 sur un réglage inobservable).

Le lot doit donc livrer, en plus de l'appel :

- **une vérification de praticabilité au déploiement**, pas au runtime : établir que l'agent `mika`
  porte un `chat_id` non nul et que le gateway répond. C'est une case de la DoD et une étape du
  runbook §3.5, pas une sonde sur le chemin critique de chaque dispatch ;
- **la trace en base comme filet nommé** : la notification est écrite **avant** la tentative
  d'envoi (ligne 60 avant ligne 73), donc une livraison morte laisse quand même la ligne dans la
  session `00000000-0000-0000-0000-700000710717`. C'est ce qui rend la halte (c) de §5.3
  décidable — présente en base et absente de Telegram sépare « le gateway est mort » de
  « l'appel n'a pas eu lieu » ;
- **l'assertion de test porte sur l'appel, et elle dit ce qu'elle ne couvre pas.** §5.1
  assertion 3 assert que `mika notify` est invoqué avec les bons arguments (binaire stubé sur
  `PATH`). Elle ne peut pas assert la livraison Telegram, qui dépend d'un état hôte hors du
  harness. Écrire cette limite dans le test lui-même évite qu'un futur lecteur prenne le vert
  pour une preuve de livraison.

**Alternative écartée** : faire remonter l'échec de livraison en rendant `mika notify` non-zéro
sur échec Telegram. C'est un changement de contrat d'une commande partagée, dont tous les autres
appelants attendent le fail-soft actuel — et ça ne servirait à rien ici, puisque le refus ne doit
de toute façon pas dépendre de l'alerte. La bonne place est la précondition de déploiement.

### 3.4 Gardes A et B — la reprise sans geste sur les tickets (R5)

**Un seul organe d'état** (`~/.mika/state/pilot-egress-down`) sert trois usages : déduplication
d'alerte, filtre `auto_pull`, détection de reprise. **Un seul sondeur** (la garde C, en shell) ;
A et B **lisent le stamp, ne sondent jamais** — dupliquer la sonde en Rust créerait un second
lecteur de la même question, ce que la maison a dû défaire une fois (`grooming_marker`, mika#2158).

**La péremption est load-bearing, pas un réglage.** Si A refuse sur stamp sans jamais re-sonder,
personne ne sonde, le stamp ne se lève jamais et la boucle est bloquée **définitivement** — le
mode de panne classique d'un disjoncteur sans ré-armement. Le stamp est donc réputé **périmé**
au-delà de `MIKA_PILOT_EGRESS_DOWN_TTL_SECS` (défaut `600`, soit un tick d'`auto_pull`) : passé ce
délai A laisse passer, la garde C re-sonde, et soit elle réussit (stamp retiré, reprise annoncée)
soit elle refuse (stamp rafraîchi, **pas** de nouvelle alerte). Le pire cas pendant une panne est
donc une tentative de dispatch toutes les 10 minutes.

**Lecture fail-open, et c'est sûr ici précisément parce que C existe** : stamp absent, illisible,
inparsable, horodatage dans le futur ⇒ traité comme « pas de panne » ⇒ le dispatch est tenté ⇒ la
garde C tranche sur une sonde fraîche. Aucune de ces lectures ne peut ouvrir le réseau.

- **Garde B** — `auto_pull` Phase 2 : nouveau filtre rendant **`Skip`**, jamais
  `SkipAndResetBudget`. Attendre un relais n'est pas un succès — c'est le compteur remis à zéro
  par l'action qu'il compte que mika#2158 a mesuré à 31 re-drives affichant 1. Nom de filtre
  (format de fil, il atterrit dans `audit_events.after_value`) : **`egress_relay_down`**.
- **Garde A** — `ready_label_handler` : nouvelle porte, placée **après** 2c (`pilot_in_flight`) et
  **avant** l'étape 3 (résolution du token), pour la raison énoncée en 3.2. Refus en `Handled`,
  jamais `Passthrough` — sans quoi `req.text` resterait sur le marqueur ready et l'INTENT_GUARD
  `webhook_ready_label_dispatch` re-sommerait le LLM de dispatcher le ticket qu'on vient de
  refuser (mika#2279). Nom de porte (format de fil `ready_label_outcome`, mika#2323) :
  **`egress_relay_down`**.

**Ordre de livraison contraint.** La garde C **seule** satisfait R1 (la sûreté) mais **casse** R5.
A et B seules ne protègent rien. Les trois vont dans le même lot ; si le lot devait être scindé,
C ne peut pas partir sans B.

### 3.5 Préalable 3 — remise en marche outillée et runbook (R6)

**Le geste existe déjà pour moitié** : `scripts/canary-pilot-containment --ensure-relay` appelle
`_ensure_pilot_egress_proxy` et imprime `relay: up` / `relay: FAILED`. Il couvre le cas « mort
simple » : le proxy `unlink` un socket orphelin avant `bind`
(`scripts/mika-pilot-egress-proxy:1300`) et gère SIGTERM pour le délier
(`:1336`), donc une relance sur socket orphelin fonctionne — c'est mesuré, et c'est ce qui rend le
fail-closed non-piégeant sur cette cause.

Ce qu'il **ne** couvre pas, et que le lot ajoute :

- **`--restart-relay`** : tuer un proxy wedgé puis relancer. Le cas « socket accepte `connect()`
  mais le proxy ne sert plus » passe la sonde de `_pilot_egress_sock_connectable` et **fait dire
  « alive » au lanceur**. C'est un angle mort réel : la garde C ne le voit pas, le refus ne se
  déclenche pas, et le pilote part derrière un relais mort. Le lot le **nomme** et lui donne un
  geste ; il ne le détecte pas (voir §6, hors périmètre).
- **`docs/operator/pilot-egress-relay.md`** : symptômes (ce que lit l'opérateur au refus, dans
  Telegram et dans le `RESULT`), diagnostic (les trois questions : binaire installé ? socket
  connectable ? log du proxy), les gestes, et la vérification. Sur le modèle de
  `docs/operator/agent-identity-reprovision.md`. **Plus une section « le canal d'alerte
  fonctionne-t-il ? »** portant la précondition de §3.3.1 : vérifier le `chat_id` de l'agent
  `mika`, émettre une notification de test, et savoir qu'un échec de livraison est muet. C'est la
  seule page où cette vérification a une chance d'être faite avant l'incident plutôt que pendant.

**Note de lecture pour le runbook** : `docs/egress-*.md` et
`crates/mika-gateway/docs/egress-search*.md` concernent l'egress **de la recherche web** (gateway,
milestone #1806). Ils n'ont **rien** à voir avec le relais d'egress du pilote. Le runbook doit le
dire en tête — deux sujets portant le même mot, dont l'un a quatre documents et l'autre aucun,
est exactement la confusion qu'un opérateur en incident fera.

---

## 4. Ce que ça coûte, nommé

**Une panne de relais arrête la boucle.** C'est le coût que la décision assume explicitement :
*« une boucle arrêtée est réversible et visible ; un egress ouvert sur du code auto-écrit ne l'est
pas. »* Ce plan ne l'adoucit pas ; il le rend bruyant (§3.3) et sans dommage collatéral sur les
tickets (§3.4).

**Deux replis voisins restent ouverts, et ils ne sont pas de même nature :**

| Repli | Ce qu'il ouvre | Statut dans ce lot |
|---|---|---|
| `MIKA_PILOT_SANDBOX=0` (`dispatch-lib.sh:183`) | tout — invocation directe | **reste ouvert**, à dessein |
| `bwrap` absent du PATH (`dispatch-lib.sh:925`) | tout — fs **et** réseau | **nommé, non fermé ici** |

Le premier est un **opt-out explicite d'opérateur**, pas un repli silencieux : quelqu'un a écrit
`0`. Le fermer serait supprimer une commande, pas corriger un défaut. Noter que lorsqu'il est
armé, `_ensure_pilot_egress_proxy` n'est jamais atteint (retour anticipé en tête de
`_run_pilot_sandboxed`) — le fail-closed ne s'y applique donc pas, et c'est cohérent.

Le second est **de la même classe que le défaut de ce ticket** : une tolérance de premier
déploiement (« first-rollout deployment tolerance », commentaire ligne 181) devenue vestige, et il
ouvre **strictement plus** que le repli d'egress. Il n'est pas fermé ici pour une raison mesurable
et non pour de la prudence : **la mesure manque.** Fermer ce repli arrête la boucle sur toute
machine sans `bwrap` (macOS, certains conteneurs), et ce plan ne sait pas si une telle machine
dispatche aujourd'hui. **Ticket de suivi**, avec son préalable écrit : établir la population des
hôtes qui dispatchent sans `bwrap`. Le fermer sans cette mesure serait échanger un fail-open
documenté contre un arrêt de rail non mesuré.

---

## 5. Contrat de vérification

### 5.1 Le test négatif de l'opérateur, littéralement

Écrit dans `skills/bundled/_shared/test-dispatch-lib.sh`, en réutilisant la sonde
`_egress_guard_probe` déjà présente (`test-dispatch-lib.sh:4428`), qui sait fabriquer les trois
états de socket (`ghost` / `live` / `absent`) et les deux états de binaire (`dies` / `missing`) :

1. **proxy arrêté ⇒ aucun pilote ne démarre** — `_run_pilot_sandboxed` rend `78` et le marqueur de
   lancement du faux pilote **n'existe pas**. C'est l'assertion porteuse : elle distingue « refusé »
   de « lancé puis échoué ».
2. **le motif est lisible** — le `RESULT` porte `CONTAINMENT REFUSAL`, la cause (`egress_bind_timeout`
   ou `egress_binary_missing`) **et** le geste de remise en marche.
3. **l'alerte part** — `mika notify` est appelé, une fois, avec `--severity escalate`
   (`mika` stubé sur `PATH` dans le test, le journal d'appels asserté). **Le test porte sur
   l'appel, jamais sur la livraison** : celle-ci dépend d'un `chat_id` hôte que le harness n'a
   pas, et `mika notify` rend `Ok` même quand Telegram échoue (§3.3.1). Cette limite est écrite
   dans le test, pour qu'un futur lecteur ne prenne pas le vert pour une preuve de livraison.
4. **une seconde tentative pendant la même panne n'alerte pas** — la déduplication mord.
5. **proxy relancé ⇒ reprise** — le dispatch suivant part, le stamp est retiré, une notification
   `info` de reprise est émise.
6. **aucun ticket n'est touché** — la garde B rend `Skip` et non `SkipAndResetBudget`
   (test Rust : `redrive_count` inchangé après un tick sur stamp frais).

**Anti-vacuité** (plan KTD6, discipline déjà appliquée par le bloc mika#2041 voisin) : les
assertions 1 et 3 doivent **échouer contre le code actuel** — aujourd'hui `rc=1` / `launched=yes` /
zéro notification. Sans cette vérification, un test peut passer sur du code mort.

### 5.2 Les quatre tests qui dépendent du fail-open — impact concret

Mesure : `grep -rn "_ensure_pilot_egress_proxy" skills/ scripts/` rend **quatre fichiers de test**
qui stubent le lanceur en `return 1` **pour exercer la branche Phase 2a** :

- `skills/bundled/_shared/tests/test_sandbox_git_usable.sh:145`
- `skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh:169`
- `skills/bundled/_shared/tests/test_sandbox_log_dir_bound.sh:110`
- `skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh:106` (via `$MOCK_EGRESS_RC`)

**Avec le fail-closed, `return 1` fait refuser le sandbox et ces quatre tests n'ont plus de
lancement à inspecter.** Ce n'est pas un détail de migration : ce sont les tests des invariants
gitdir (mika#2141), credential GitHub (mika#2056), journal de session (mika#2165) et argv sans
secret (mika#2039). Chacun doit être migré vers un stub `return 0`, ce qui suppose de **fabriquer
un relais servant** dans le harness plutôt que de s'appuyer sur son absence. Le travail est réel et
il est dans le lot ; l'omettre laisserait quatre invariants de contenance non testés — c'est-à-dire
affaiblirait la contenance au nom d'un correctif de contenance.

`scripts/canary-pilot-containment:179` appelle le lanceur en `|| true` et ne lit pas le code de
retour : contrat inchangé, pas d'impact. `:90` (`--ensure-relay`) lit le code de retour, dont la
sémantique ne change pas.

### 5.3 Sondes post-déploiement, avec leurs haltes

**(a) Le rail tourne toujours.** Sur 48 h, aucun `CONTAINMENT REFUSAL` de cause egress dans les
`RESULT`, et le volume de dispatches est inchangé. **Régime attendu : zéro refus.** Un refus est
un résultat, pas une panne du correctif — il dit que le relais était mort et que le pilote ne
serait pas parti protégé.

**(b) Halte — refus soutenus.** Si les refus deviennent le régime nominal, **ne pas désarmer et
ne pas rallonger la fenêtre de bind** : le relais est réellement instable, et c'est *lui* qu'il
faut traiter. Le lecteur est `/var/log/mika/pilot-egress-proxy.log`, l'instrument de diagnostic sur
lequel mika#2041 puis mika#2051 se sont appuyés.

**(c) Halte — l'alerte n'arrive pas alors qu'un refus a eu lieu.** L'ordre de lecture est imposé
par le fait que l'échec de livraison est muet (§3.3.1) : la commande a rendu `0` dans **tous** les
cas ci-dessous, donc son code de retour ne discrimine rien. Vérifier **d'abord** que la
notification est en base :
`SELECT * FROM messages WHERE session_id = '00000000-0000-0000-0000-700000710717' ORDER BY created_at DESC LIMIT 5;`

- **Absente de la base** ⇒ `mika notify` n'a pas été appelé du tout : lire le site d'appel shell.
- **Présente en base, absente de Telegram, `chat_id` présent et non nul** ⇒ le défaut est dans la
  livraison gateway, pas dans ce lot.
- **Présente en base, absente de Telegram, `chat_id` absent ou nul** ⇒ **le canal n'a jamais été
  appairé** et la case de praticabilité de la DoD n'a pas été faite. C'est la cause la plus
  probable d'une première alerte perdue, et le remède est un appairage, pas une correction de
  code. Ne pas chercher le défaut dans la garde C.

**(d) Halte — un ticket parqué malgré la garde B.** Lire
`SELECT after_value, count(*) FROM audit_events WHERE tool_name = 'auto_pull_exclusion' GROUP BY 1;`
La présence d'`egress_relay_down` prouve que B mord ; son absence pendant une panne dit que le
stamp n'est pas lu (chemin, péremption, `MIKA_HOME`) — **réparer la lecture, ne pas allonger le
budget de re-drive**, qui masquerait le symptôme sans toucher la cause.

**(e) Contrôle négatif du déploiement.** L'absence de refus ne prouve pas que la garde est en
vigueur : elle est identique à l'absence de panne. Pour établir le déploiement, exercer
`scripts/canary-pilot-containment --restart-relay` et vérifier la reprise — et se rappeler qu'un
binaire antérieur au correctif produit exactement le même silence (classe mika#2340).

---

## 6. Hors périmètre, délibérément

- **Le relais wedgé** (accepte `connect()`, ne sert plus). Nommé en §3.5, doté d'un geste
  (`--restart-relay`), **non détecté**. Le détecter demande une sonde applicative — une requête de
  bout en bout à travers le proxy — sur le chemin critique de chaque dispatch, dont le coût et le
  taux de faux positifs n'ont pas été mesurés. **Ticket de suivi**, préalable : une mesure de la
  latence d'une telle sonde.
- **La cause de la casse du 15/09** — non établie (§1.4). Ce lot rend la panne bruyante et sans
  dommage ; il ne la fait pas disparaître.
- **Le repli `bwrap` absent** — §4, avec son préalable de mesure.
- **`MIKA_PILOT_SANDBOX=0`** — §4, opt-out explicite, conservé.
- **L'allowlist de noms d'hôtes elle-même** (`scripts/mika-pilot-egress-proxy`). Ce ticket porte
  sur ce qui arrive **quand le contrôle ne démarre pas**, jamais sur ce que le contrôle autorise.

---

## Definition of Done

- [ ] `_ensure_pilot_egress_proxy` pose un motif structuré ; `_run_pilot_sandboxed` refuse
      (`return 78`) au lieu de retomber en Phase 2a, **avant** `_stage_pilot_gh_token`.
- [ ] Le `RESULT` de refus nomme la cause et le geste de remise en marche.
- [ ] `mika notify --channel telegram --severity escalate` est émis au premier refus d'un épisode,
      en `|| true`, et une notification `info` annonce la reprise.
- [ ] **Praticabilité du canal établie au déploiement** (§3.3.1) : l'agent `mika` porte un
      `chat_id` non nul en `customer_config` et une notification de test est reçue. Sans cette
      case, l'AC1 livre un appel émis et non une escalade lue.
- [ ] `~/.mika/state/pilot-egress-down` est écrit/retiré par la garde C, avec péremption
      `MIKA_PILOT_EGRESS_DOWN_TTL_SECS` (défaut 600).
- [ ] Garde B (`auto_pull` Phase 2, filtre `egress_relay_down`, verdict `Skip`) et garde A
      (`ready_label_handler`, porte `egress_relay_down`, refus `Handled`) livrées dans le même lot.
- [ ] `scripts/canary-pilot-containment --restart-relay` livré.
- [ ] `docs/operator/pilot-egress-relay.md` livré, avec la note de désambiguïsation « egress
      pilote ≠ egress recherche ».
- [ ] Les six assertions de §5.1 passent, et les assertions 1 et 3 échouent contre `HEAD`.
- [ ] Les quatre tests de §5.2 sont migrés et verts.
- [ ] `make test`, `cargo clippy`, `cargo fmt`, `make verify-bundled-skills` verts.
- [ ] Les deux nouveaux noms de fil (`egress_relay_down` en filtre et en porte) sont épinglés par
      les scans existants (`mika2131_filter_names_are_a_wire_format`,
      `mika2323_gate_names_are_a_wire_format`).
- [ ] Le commentaire `dispatch-lib.sh:488-490` est remplacé par la **décision datée** (option 1,
      2026-09-20, Vincent après bearing de Prime) et non par une nouvelle justification implicite.
- [ ] `CLAUDE.md` — entrée `MIKA_PILOT_EGRESS_DOWN_TTL_SECS` et section de lecture opérateur.

## Acceptance criteria

*Le ticket #2049 n'a pas de section `## Acceptance criteria` ; les critères ci-dessous sont
transcrits du commentaire opérateur du 2026-09-20T15:04:30Z (les trois préalables et le test
négatif), et AC5 est dérivé de la clause « sans intervention sur les tickets » de ce même test.*

**AC1 — Alerte active au refus.** Un refus pour cause d'egress déclenche une escalade sur un canal
réellement lu, par un chemin déterministe sans tour LLM. Vérifiable en **deux** moitiés, parce
qu'aucune ne suffit seule (§3.3.1) : *l'appel* par §5.1 assertion 3 (test automatisé), *la
lecture effective* par la case de praticabilité de la DoD (`chat_id` appairé + notification de
test reçue), vérifiée au déploiement. Un test vert seul atteste un appel émis — c'est-à-dire
exactement ce que le préalable opérateur distingue d'une escalade.

**AC2 — Diagnostic dans le motif de refus.** Le dispatch refusé dit **pourquoi** (cause distincte
par mode de panne) et **comment relancer**. Vérifiable : §5.1 assertion 2.

**AC3 — Remise en marche testée du relais.** Un geste outillé de redémarrage supervisé **et** un
runbook, avec un test. Vérifiable : `--restart-relay` + `docs/operator/pilot-egress-relay.md` +
§5.1 assertion 5.

**AC4 — Test négatif.** Proxy arrêté ⇒ **aucun pilote ne démarre**, l'alerte part, le motif est
lisible ; proxy relancé ⇒ le dispatch reprend. Vérifiable : §5.1 assertions 1, 2, 3, 5.

**AC5 — Reprise sans intervention sur les tickets.** Aucun ticket n'est parqué
(`operator-review`) ni n'a consommé de budget de re-drive du fait de la panne. Vérifiable : §5.1
assertion 6.

**AC6 — Aucune régression de contenance.** Les quatre invariants de §5.2 restent testés et verts
après migration.

**AC7 — Pas d'échappatoire.** Aucune variable d'environnement ne lève le refus (option 2 écartée
par décision opérateur). Vérifiable : absence de toute lecture d'environnement dans la branche de
refus de la garde C.
