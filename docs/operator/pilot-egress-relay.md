# Runbook — le relais d'egress du pilote

> **Ne confondez pas deux sujets qui portent le même mot.** Ce document porte sur
> le **relais d'egress du pilote** — le proxy hôte qui coupe le réseau du sandbox
> claude-pilot et n'autorise qu'une allowlist de noms d'hôtes. Il n'a **rien** à
> voir avec l'**egress de la recherche web**, documenté par `docs/egress-*.md` et
> `crates/mika-gateway/docs/egress-search*.md` (gateway, milestone #1806). L'un a
> quatre documents et l'autre n'en avait aucun ; c'est exactement la confusion
> qu'un opérateur fera en incident, et c'est pourquoi cette note est en tête.

## 1. Ce qui a changé, et ce que ça coûte

Depuis le 2026-09-20 (mika#2049, décision de Vincent après bearing de Prime), le
repli d'egress est **fail-closed** : relais indisponible ⇒ **le dispatch est
refusé**, avec un motif explicite. Le pilote ne part jamais sans sa coupure
réseau.

Avant cette date, `dispatch-lib` retombait en « Phase 2a » — coupure du système
de fichiers conservée, **réseau ouvert** — et lançait le pilote quand même. La
justification écrite était la fenêtre de déploiement de #1894, close depuis
longtemps ; la posture, elle, était restée.

**Le coût est assumé et il est réel : une panne de relais arrête la boucle.** Le
motif de la décision, mot pour mot : *« une boucle arrêtée est réversible et
visible ; un egress ouvert sur du code auto-écrit ne l'est pas. »* L'option d'une
variable d'échappement a été **écartée par écrit** : *« elle recrée le fail-open
sous un autre nom, et un WARN sous charge n'est lu par personne. »* Il n'existe
donc aucune variable d'environnement qui lève le refus.

**Ce que la panne ne coûte PAS : vos tickets.** Pendant une panne, les tickets
`ready` sont *sautés*, pas re-drivés : aucun ne consomme son budget de re-drive,
aucun n'est parqué derrière `operator-review`. À la reprise du relais, la boucle
repart **sans aucun geste sur les tickets**.

## 2. Symptômes — ce que vous lisez quand ça arrive

### Dans Telegram (canal d'escalade)

Une notification, **une seule par épisode de panne** :

```
[escalate] 🚨 Egress relay DOWN — pilot dispatch refused (fail-closed, mika#2049).
Cause: egress_bind_timeout. The egress relay did not bind /tmp/mika-pilot-egress.sock
within 3s. Restart it with `scripts/canary-pilot-containment --restart-relay` […]
Tickets are NOT being parked; the loop resumes on its own once the relay serves again.
```

Et une à la reprise :

```
[info] ✅ Egress relay back up — pilot dispatch resumed (mika#2049). No action needed on tickets.
```

### Dans le `RESULT` de la tâche

```
CONTAINMENT REFUSAL (exit 78) — the pilot was never launched.

the host egress relay is not serving, so the pilot would have run with filesystem
containment only and an OPEN NETWORK — which is the posture mika#2049 closed on
2026-09-20 (operator decision: fail-closed, no escape hatch).

Cause: egress_bind_timeout
Remedy: […]

This is not pilot drift and not a pipeline failure […]
```

### Dans les journaux

```bash
# Le refus, côté dispatch (un fichier par dispatch)
grep -l '^dispatch-lib: refusing to launch the pilot — egress relay unavailable' \
    "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr

# La cause, avec son token stable — un par cause depuis mika#2049
grep -l '^dispatch-lib: pilot_egress_guard.binary_missing' \
    "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr   # le binaire n'est pas déployé
grep -l '^dispatch-lib: pilot_egress_guard.unreachable' \
    "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr   # le relais ne bind pas

# La reprise
grep -l '^dispatch-lib: pilot_egress_guard.recovered' \
    "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr

# Côté moteur (mika-spirit)
grep auto_pull_egress_relay_down   "$MIKA_SPIRIT_LOG_FILE" | jq '{motif, age_secs}'
grep ready_label_egress_relay_down "$MIKA_SPIRIT_LOG_FILE" | jq '{repo, num, motif}'
```

**Ancrez sur `^dispatch-lib: `.** Le fichier `.stderr` d'un dispatch porte aussi
la prose du pilote lui-même, horodatée en tête de ligne ; une session qui *parle*
de ces signaux produirait un faux positif sur un grep nu. Toute émission réelle
commence en colonne 0 par `dispatch-lib: ` (leçon mika#2050).

### En SQL

```sql
-- Combien de tickets la panne a-t-elle épargnés (auto_pull) ?
SELECT target_key, count(*) FROM audit_events
 WHERE tool_name = 'auto_pull_exclusion' AND after_value = 'egress_relay_down'
 GROUP BY 1 ORDER BY 2 DESC;

-- Combien d'événements `ready` ont été refusés à la porte ?
SELECT target_key, count(*) FROM audit_events
 WHERE tool_name = 'ready_label_egress_relay_down' GROUP BY 1 ORDER BY 2 DESC;

-- La trace de l'alerte, même si Telegram n'a rien livré (voir §5)
SELECT created_at, content FROM messages
 WHERE session_id = '00000000-0000-0000-0000-700000710717'
 ORDER BY created_at DESC LIMIT 5;
```

## 3. Diagnostic — trois questions, dans cet ordre

**Q1 — le binaire est-il installé ?**

```bash
ls -l ~/.local/bin/mika-pilot-egress-proxy
```

Absent ou non exécutable ⇒ motif `egress_binary_missing`. Remède : `make install`
sur l'hôte de dispatch. **Ce n'est pas une panne du relais, c'est un déploiement
incomplet** — et c'est la cause que le token `pilot_egress_guard.unreachable` ne
couvre *pas* (mika#2050 : un opérateur qui prend ce token pour le prédicat lit un
régime nominal sur une flotte dont le binaire n'a jamais été déployé).

**Q2 — le socket est-il connectable ?**

```bash
ls -l /tmp/mika-pilot-egress.sock
python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.settimeout(1); s.connect(sys.argv[1]); s.close(); print("connectable")' /tmp/mika-pilot-egress.sock
```

Le fichier peut exister sans que personne n'écoute : un `kill` ne délie pas un
socket unix, et le chemin survit à son propriétaire en orphelin. **Tester
l'existence du fichier ne répond pas à la question** — c'est précisément ce qui a
laissé passer l'incident mika#2041.

- socket absent ou refusant `connect()` ⇒ relais mort. Voir §4.1.
- socket acceptant `connect()` **mais les dispatches échouent quand même** ⇒
  relais **wedgé**. Voir §4.2. C'est l'angle mort : la sonde du lanceur est un
  `connect()`, donc un relais wedgé lui répond « vivant ».

**Q3 — que dit le journal du proxy ?**

```bash
tail -50 "${MIKA_PILOT_EGRESS_LOG_DIR:-/var/log/mika}/pilot-egress-proxy.log"
# et, si ce répertoire n'est pas accessible en écriture, le repli :
tail -50 /tmp/mika-pilot-egress-proxy.log
```

**Les deux chemins sont nommés délibérément.** Ne chercher que le premier est la
façon dont on conclut « pas de journal, donc rien n'a tourné ».

#### Q3.1 — lire ce journal : la table des signatures

`tail` montre les lignes ; il ne dit pas ce qu'elles veulent dire. Le lecteur :

```bash
grep -E 'pilot_egress_startup|host-unix listening on' \
    "${MIKA_PILOT_EGRESS_LOG_DIR:-/var/log/mika}/pilot-egress-proxy.log"
```

Trois lignes possibles par lancement, et c'est leur **combinaison** qui tranche :

| Ce qu'on lit pour **un** lancement | Lecture |
|---|---|
| `.begin` **puis** `host-unix listening on` | sain — ce proxy a bindé |
| `.begin`, **pas** de `.signalled`, **pas** de `listening` | mort dans la fenêtre pré-bind par un signal **non rattrapable** — SIGKILL ou OOM-kill |
| `.begin` **puis** `.signalled <SIG>` (sortie `3`) | un SIGTERM/SIGINT a atterri pendant le démarrage — **le signal est nommé sur la ligne** |
| **aucune** ligne `.begin` pour ce lancement | mort **avant** que Python ne tourne — échec d'`exec`, interpréteur, dépendance manquante |

Trois points, sans lesquels la table conduit à la mauvaise conclusion :

- **La quatrième branche est une information, pas un silence.** `.begin` est la
  **première** ligne du processus, émise avant `bind()` et juste après l'armement
  des handlers — contrat tenu par
  `scripts/test-pilot-egress-proxy-status.py::test_startup_emits_a_begin_breadcrumb_before_bind`.
  Son absence pour un lancement donné dit donc quelque chose de précis. **À ne
  pas confondre avec un journal *absent***, qui est le cas « binaire jamais
  déployé » que la Q1 traite déjà (classe mika#2340). Le jour où `.begin` cesse
  d'être la première ligne, c'est ce test qui rougit, pas cette table qui dérive.
- **La jointure se fait par le pid, pas par l'horodatage.** Le journal du proxy
  est **cumulatif** ; le `.stderr` du dispatch est **par dispatch**. Le pid que
  porte `pilot_egress_guard.unreachable … (pid N)` côté dispatch est le même que
  celui du `pid=N` de la ligne `.begin` côté proxy. Corréler à la montre — ce
  qu'il fallait faire avant mika#2051 — est ce qui a rendu le diagnostic du
  2026-08-29 coûteux.
- **C'est un geste *hôte*.** Un pilote dispatché ne voit pas `/var/log/mika/` :
  le bac à sable `bwrap` ne le monte pas (classe mika#2165). **L'absence de ce
  fichier depuis une session de dispatch n'est pas un résultat** et ne doit
  jamais être lue comme « aucune récurrence ».

**Ce que cette table ne dit pas, et ne prétend pas dire : *qui* a envoyé le
signal.** Guardrail, timeout, démantèlement de groupe de processus, OOM — la
mesure est hors dépôt. mika#2051 livre de quoi **attribuer** la prochaine
occurrence, pas une cause ; si aucune récurrence n'est mesurée, la conclusion
est « instrumenté, sans récurrence », jamais « cause identifiée ».

## 4. Les gestes

### 4.1 Relais mort — le relancer

```bash
scripts/canary-pilot-containment --ensure-relay
```

Idempotent. Le proxy délie lui-même un socket orphelin avant `bind()` et gère
`SIGTERM` pour le délier en sortant, donc une relance sur socket orphelin
fonctionne — c'est mesuré, et c'est ce qui rend le fail-closed non piégeant sur
cette cause.

### 4.2 Relais wedgé — le redémarrer

```bash
scripts/canary-pilot-containment --restart-relay
```

Tue le détenteur du socket (par identité de socket via `fuser`, **jamais** par
`pkill -f`, qui attraperait aussi votre éditeur et ce script lui-même), délie le
chemin, puis relance.

**Ce geste ne détecte pas l'état wedgé, il le traite.** Le détecter demanderait
une sonde applicative de bout en bout sur le chemin critique de chaque dispatch,
dont le coût et le taux de faux positifs ne sont pas mesurés — hors périmètre de
mika#2049, ticket de suivi.

### 4.3 Vérifier

```bash
scripts/canary-pilot-containment --ensure-relay   # doit imprimer `relay: up (…)`
```

Puis attendre un dispatch. **Aucun geste n'est dû sur les tickets** : le prochain
dispatch réussi retire le marqueur et annonce la reprise. Le délai est borné par
`MIKA_PILOT_EGRESS_DOWN_TTL_SECS` (défaut 1800 s), donc **au plus 30 minutes**.

Si vous ne voulez pas attendre, un `ready` reposé à la main (`remove` **puis**
`add` — GitHub n'émet `issues.labeled` que sur une transition) déclenche un
dispatch immédiatement.

## 5. Le canal d'alerte fonctionne-t-il ? — à faire AVANT l'incident

**C'est la section dont l'absence coûte l'alerte entière**, et elle n'a de chance
d'être exécutée qu'ici, à froid.

`mika notify` rend `Ok(())` **même quand la livraison Telegram échoue** :
l'échec est attrapé, écrit sur son propre stderr, et avalé. Seul un échec
d'écriture en base rend non-zéro. **L'appelant shell ne peut donc
structurellement pas savoir si l'alerte a atteint quelqu'un**, et aucun code
supplémentaire côté dispatch ne changerait cela. Ce qui rend le canal réel est
une **précondition de déploiement**, vérifiée ici.

### 5.1 L'agent `mika` porte-t-il un `chat_id` ?

La livraison lit `get_customer_config("chat_id")` sur l'agent **`mika`** —
jamais sur `mika-dev`, qui est pourtant l'agent du dispatch. Absent ⇒ `bail!`
(« no Telegram pairing yet ») ⇒ avalé.

```bash
mika config get chat_id --agent mika
```

Vide ou nul ⇒ **le canal n'a jamais été appairé** et l'escalade sera silencieuse
de bout en bout. Appairez avant d'avoir besoin de l'alerte.

### 5.2 `MIKA_INTERNAL_TOKEN` est-il résolu ?

Lu depuis `ctx.settings` (donc via `~/.mika/.env`), et non depuis
l'environnement du process — ce qui le rend **insensible** au
`scrub_mika_env_vars` qui retire tous les `MIKA_*` des enfants de dispatch. Cette
moitié-là est saine ; c'est la 5.1 qui est fragile.

### 5.3 Émettre une notification de test

```bash
mika notify --channel telegram --severity info \
    --text "Test du canal d'escalade egress (mika#2049) — ignorez."
```

Vous devez la recevoir **dans Telegram**. Si elle n'arrive pas, vérifiez qu'elle
est au moins en base (requête SQL de la §2) : la notification est écrite **avant**
la tentative d'envoi, donc une livraison morte laisse quand même la ligne.

### 5.4 Ordre de lecture quand une alerte manque

Le code de retour ne discrimine rien (il vaut `0` dans tous les cas ci-dessous).
Vérifiez **d'abord** la base :

| État | Lecture | Remède |
|---|---|---|
| Absente de la base | `mika notify` n'a pas été appelé du tout | lire le site d'appel shell |
| En base, absente de Telegram, `chat_id` présent | défaut de livraison gateway | hors périmètre mika#2049 |
| En base, absente de Telegram, `chat_id` absent | **le canal n'a jamais été appairé** | §5.1 — un appairage, pas un correctif de code |

**La troisième ligne est la cause la plus probable d'une première alerte perdue.
Ne cherchez pas le défaut dans la garde.**

## 6. Réglage

| Variable | Défaut | Rôle |
|---|---|---|
| `MIKA_PILOT_EGRESS_DOWN_TTL_SECS` | `1800` | péremption du marqueur de panne |
| `MIKA_PILOT_EGRESS_LOG_DIR` | `/var/log/mika` | où le proxy journalise |

**Le TTL n'est pas libre.** Il doit rester **strictement supérieur** au seuil de
stuck-ready (`MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS`, défaut 900 s), avec
marge. À `TTL < seuil`, le marqueur est périmé **à chaque fois que la Phase 2
d'`auto_pull` le regarde**, la garde qui épargne vos tickets n'est jamais
consultée avec un marqueur frais, et une panne d'une heure parque chaque ticket —
sans qu'aucun test ne rougisse. La relation est tenue par
`auto_pull::tests::mika2049_le_ttl_du_marqueur_depasse_le_seuil_de_stuck_ready`.

## 7. Haltes

**Halte 1 — des refus soutenus deviennent le régime nominal.** **Ne désarmez pas
et n'allongez pas la fenêtre de bind** : le relais est réellement instable, et
c'est *lui* qu'il faut traiter. Le lecteur est
`/var/log/mika/pilot-egress-proxy.log`, l'instrument sur lequel mika#2041 puis
mika#2051 se sont appuyés.

**Halte 2 — un ticket est parqué (`operator-review`) malgré la garde.** Deux
causes produisent un symptôme rigoureusement identique, et il faut les départager
**dans cet ordre** :

1. **Le marqueur n'est pas là où le moteur regarde.** Décidable en une commande :
   ```bash
   ls -la "$HOME/.mika/state/pilot-egress-down"
   # et la même sous $MIKA_HOME si la variable est posée sur le service
   ```
   Deux chemins distincts ⇒ c'est la divergence de frontière (le shell est
   `scrub`é de `MIKA_HOME`, le moteur ne l'est pas), et **le remède est le
   chemin, pas le réglage**.
2. **Le marqueur est systématiquement périmé quand Phase 2 regarde.** Comparez
   `MIKA_PILOT_EGRESS_DOWN_TTL_SECS` à
   `MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS` — voir §6.

L'ordre est imposé par le coût de l'erreur : chercher un défaut de TTL sur une
divergence de chemin conduit à **rallonger** le TTL, ce qui ne répare rien et
ajoute de la latence de reprise. Dans tous les cas, **réparez la lecture,
n'allongez pas le budget de re-drive**, qui masquerait le symptôme sans toucher
la cause.

**Halte 3 — un refus est annoncé comme un succès.** Si une notification
« claude-pilot completed » porte un ticket dont le `RESULT` contient
`CONTAINMENT REFUSAL`, ou si une tâche reste `in_progress` sur « awaiting QA
review » sans PR : vérifiez **d'abord** que le discriminant de
`self-dev-callback` est écrit en `contains` et non en `starts with` — le `RESULT`
commence par `Log path:`, donc un `starts with` recopié par analogie ne matche
jamais. **Ne répondez pas par une garde EndTurn** : son lexique (« completed »,
« PR », « awaiting QA ») est le vocabulaire nominal de *tous* les callbacks sains.

**Halte 4 — l'absence de refus ne prouve pas que la garde est en vigueur.** Elle
est identique à l'absence de panne. Pour établir le déploiement, exercez
`scripts/canary-pilot-containment --restart-relay` et vérifiez la reprise — et
rappelez-vous qu'un binaire antérieur au correctif produit exactement le même
silence (classe mika#2340).

## 8. Ce qui reste ouvert, nommé

- **Le relais wedgé n'est pas détecté** (§4.2). Il a un geste, pas une sonde.
- **La cause de la casse du 15/09 n'est pas établie.** Mesure du 2026-09-20 :
  `/tmp/mika-pilot-egress.sock` portait une mtime du 15 septembre 06:41 et le
  relais servait normalement depuis. Ce lot rend la panne bruyante et sans dommage
  collatéral ; il ne la fait pas disparaître.
- **`MIKA_PILOT_SANDBOX=0`** reste un opt-out d'opérateur explicite — quelqu'un a
  écrit `0`. Quand il est armé, la garde d'egress n'est jamais atteinte, et c'est
  cohérent : ce n'est pas un repli silencieux.
- **`bwrap` absent du `PATH`** ouvre **strictement plus** que le repli d'egress
  (fs *et* réseau) et reste ouvert. Il n'est pas fermé ici pour une raison
  mesurable et non par prudence : **la mesure manque** — fermer ce repli
  arrêterait la boucle sur toute machine sans `bwrap` (macOS, certains
  conteneurs), et on ne sait pas si une telle machine dispatche aujourd'hui.
  Ticket de suivi, avec son préalable écrit : établir la population des hôtes qui
  dispatchent sans `bwrap`.
