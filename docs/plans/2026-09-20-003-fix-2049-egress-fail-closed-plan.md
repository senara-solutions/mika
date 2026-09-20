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

### 3.1 Quatre gardes, et une seule d'entre elles protège

C'est la propriété centrale du lot, et la confondre affaiblirait la protection :

| | Où | Rôle | Lit un état persistant ? | Fail-* |
|---|---|---|---|---|
| **C** | `dispatch-lib::_run_pilot_sandboxed` | **la protection** — refuse le lancement | **non**, sonde à chaque fois | fail-**closed** |
| **B** | `auto_pull` Phase 2 (Rust) | économie — ne consomme pas de budget de re-drive | oui (stamp) | fail-**open** |
| **A** | `ready_label_handler` (Rust) | économie — ne crée pas de tâche ni de différé | oui (stamp) | fail-**open** |
| **D** | `self-dev-callback` (prompt) | **honnêteté** — le refus n'est pas rapporté comme un succès | non | — |

**A et B sont des optimisations de confort opérateur, faillibles et fail-open ; C est la
protection, inconditionnelle, et ne lit aucun état persistant.** Un futur lecteur tenté de durcir
A ou B au motif qu'elles sont fail-open doit savoir que la sûreté ne repose pas sur elles : elle
repose sur C, qui sonde le socket à chaque dispatch. Inversement, quiconque affaiblirait C en
lui faisant lire le stamp de A/B transformerait la protection en cache, et un cache périmé est
précisément un fail-open avec une étape de plus.

**D ne protège rien et n'est pas optionnelle pour autant** (§3.5) : sans elle, le refus que C
produit est classé *succès* par le callback, et le lot livrerait une protection qui ment sur son
propre déclenchement.

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
mika#2141 déjà câblé : `_PILOT_SANDBOX_REFUSAL=<texte> ; return 78`. `_run_claude_pilot` classe
déjà le code 78 en `CONTAINMENT REFUSAL (exit 78) — the pilot was never launched`
(`dispatch-lib.sh:2661`, le motif interpolé à `:2663`), texte qui dit explicitement que ce n'est
ni une dérive du pilote ni un échec de pipeline. Il reste à enrichir le motif du **geste de
remise en marche** (R2).

**Le canal est câblé jusqu'au `RESULT`, et pas au-delà — c'est là que le « gratuitement » se
paie.** Une rédaction antérieure de ce plan disait que ce choix livrait le préalable 2 « presque
intégralement et gratuitement ». La moitié *texte* est effectivement gratuite ; la moitié
*classification* ne l'est pas, et la §3.5 la chiffre. Le lot ne peut pas s'appuyer sur la
réutilisation de mika#2141 sans reprendre aussi ce qui, chez mika#2141, était resté dormant
faute d'être emprunté.

**Et la queue du texte emprunté dit le contraire du motif qu'on y insère.** Le bloc de
`dispatch-lib.sh:2661-2668` ne se réduit pas au motif interpolé : il se **termine** par deux
phrases fixes, écrites pour les deux causes gitdir de mika#2141 —

```
Fix the worktree, then re-dispatch.
```

Pour un refus d'egress, **le worktree est sain** et ce qu'il faut réparer est le relais. Un
implémenteur qui se contente d'enrichir `_PILOT_SANDBOX_REFUSAL` — le geste que la phrase
précédente rend naturel — livre un `RESULT` qui nomme le relais en son milieu et prescrit de
réparer le worktree à sa fin. C'est une contradiction **dans le seul texte que l'AC2 rend
lisible**, et elle oriente l'opérateur vers le mauvais organe au moment précis où il lit vite.
La queue doit donc suivre la cause — soit en la déplaçant dans `_PILOT_SANDBOX_REFUSAL` chez
les deux appelants gitdir, soit en la conditionnant. La première branche est préférable :
elle rend le texte de refus **entièrement** porté par le motif, donc extensible sans retoucher
ce bloc au prochain refus de contenance.

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
  runbook §3.6, pas une sonde sur le chemin critique de chaque dispatch ;
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
au-delà de `MIKA_PILOT_EGRESS_DOWN_TTL_SECS` : passé ce délai A laisse passer, la garde C
re-sonde, et soit elle réussit (stamp retiré, reprise annoncée) soit elle refuse (stamp rafraîchi,
**pas** de nouvelle alerte). Le pire cas pendant une panne est donc une tentative de dispatch par
TTL écoulé.

**Le défaut de ce TTL n'est pas libre** : il est contraint par le seuil de re-drive, et le
dimensionner sur la cadence du tick d'`auto_pull` rend cette garde inatteignable. C'est
l'objet de §3.4.1, à lire avant de toucher à la valeur.

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

#### 3.4.1 La péremption doit dépasser le seuil de re-drive, sinon la garde B ne mord jamais

**C'est un défaut d'une rédaction antérieure de ce plan, et il annulait sa §1.3.** Le TTL y était
posé à `600` s « soit un tick d'`auto_pull` », en raisonnant sur la *cadence du tick*. Le nombre
qui gouverne n'est pas celui-là : c'est `STUCK_READY_THRESHOLD_DEFAULT_SECS = 900`
(`auto_pull.rs:91`), l'âge que le label `ready` doit atteindre pour que Phase 2 re-drive. Avec
`TTL = 600 < 900`, **le stamp est périmé à chaque fois que Phase 2 le regarde**, et l'arithmétique
se déroule ainsi (Phase 2 re-drive par `remove` → `add`, ce qui remet l'âge du label à zéro) :

| t | événement | `redrive_count` |
|---|---|---|
| 0 | panne ; dispatch refusé ; stamp écrit | 0 |
| 900 | âge label = 900 ≥ seuil ; stamp vieux de 900 > 600 ⇒ **périmé, B laisse passer** ; re-drive ; C refuse ; stamp rafraîchi | 1 |
| 1800 | idem | 2 |
| 2700 | idem | 3 |
| 3600 | `redrive_count` = 3 ≥ `MAX_REDRIVES_DEFAULT` (`auto_pull.rs:106`) ⇒ **abandon : `operator-review` posé, `ready` retiré** | — |

**Une panne d'environ une heure parque donc chaque ticket — exactement le résultat que §1.3
impute au monde *sans* garde B.** La garde, à ce dimensionnement, ne change pas une ligne du
calcul : elle n'est jamais consultée avec un stamp frais sur le seul chemin qu'elle existe pour
couvrir. R5 et AC5 resteraient ouverts, et la sonde (d) de §5.3 chercherait `egress_relay_down`
dans `audit_events` sans jamais l'y trouver.

**L'invariant à livrer, et il s'écrit comme un invariant, pas comme deux nombres.**
`MIKA_PILOT_EGRESS_DOWN_TTL_SECS` doit rester **strictement supérieur** au seuil de stuck-ready
effectif, avec marge. Défaut proposé : **`1800`** (deux fois le seuil). Le couplage est
load-bearing et doit être écrit là où il se lit — dans le doc-comment de la constante, nommant
`STUCK_READY_THRESHOLD_ENV` — faute de quoi quelqu'un baissera l'un des deux et rouvrira ce trou
en silence. Précédent maison exact : mika#2362, où une enveloppe multiple exact du plafond rendait
la dernière tentative nominale **inatteignable** sans qu'aucun test ne rougisse, parce que la
relation entre les deux nombres n'était écrite nulle part.

**Ce que le nouveau défaut coûte, nommé.** Le TTL borne la latence de reprise : le stamp n'est
retiré que par un dispatch réussi, donc après le retour du relais la boucle repart en **au plus
30 min** au lieu de 10. C'est conforme au test négatif de l'opérateur, qui exige une reprise
*sans intervention sur les tickets* — jamais une reprise instantanée — et c'est le bon côté de
l'arbitrage : trente minutes d'attente contre un ticket parqué qui, lui, exige un geste humain.

**Alternative écartée** : faire rendre `Skip` à la garde B sur stamp présent **même périmé**. Elle
ferme le tableau ci-dessus, mais rend le déblocage dépendant d'un dispatch qui ne viendra jamais —
A et B bloquant tous les chemins, plus rien ne re-sonde et le stamp ne se lève pas. C'est le
blocage définitif que §3.4 vient d'écarter, réintroduit par l'autre bout.

#### 3.4.2 Le stamp traverse une frontière que rien n'a encore traversée, et les deux côtés ne résolvent pas le même chemin

**C'est le second défaut de dimensionnement de ce plan, et il annule AC5 aussi sûrement que le
premier — mais sans qu'aucun test puisse rougir.** §3.4 pose « un seul organe d'état » sans
établir que ses deux extrémités désignent le même fichier. Elles ne le font pas.

**Mesure.** `scrub_mika_env_vars` (`crates/mika-agent/src/skills/executor.rs:40`) retire de
l'enfant de dispatch **toute** variable commençant par `MIKA_` — `MIKA_HOME` comprise. Côté Rust,
la résolution du home est `$MIKA_HOME > ~/.mika` (`crates/mika-common/src/home.rs:326-328`). Donc :

| | Process | Résout | Sur une installation posant `MIKA_HOME` |
|---|---|---|---|
| Garde C (écrit) | enfant de dispatch, **scrubé** | `$HOME/.mika/state/` | `$HOME/.mika/state/` |
| Gardes A et B (lisent) | mika-spirit | `global_home/state/` | **`$MIKA_HOME/state/`** |

Le stamp est alors écrit à un endroit et cherché à un autre. **La protection tient** — la garde C
ne lit aucun stamp et refuse sur une sonde fraîche — mais A et B, fail-open par construction
(§3.4), lisent « pas de panne » et laissent passer chaque dispatch. La garde B ne rend jamais
`egress_relay_down`, le budget de re-drive se consomme, et le tableau de §3.4.1 se rejoue
intégralement : **une panne d'environ une heure parque chaque ticket.** R5 et AC5 sont faux en
production pendant que la suite de tests est verte, puisqu'un harness pose le même home des deux
côtés ou n'en pose aucun.

**Le mode de panne est muet du côté rassurant.** Une divergence de chemin ne peut pas ouvrir le
réseau — elle ne peut acheter que de l'inertie, jamais un faux positif. C'est l'asymétrie exacte
que mika#2249 a dû écrire pour le couple `MIKA_PILOT_LOG_DIR` / `PILOT_LOG_DIR`, et c'est ce qui
rend le défaut **supportable mais invisible** : rien ne casse, la boucle a l'air de fonctionner,
et seuls des tickets parqués en témoignent — c'est-à-dire le symptôme que l'opérateur a
explicitement demandé d'exclure.

**Aucune convention à suivre : ce stamp serait le premier de son espèce.** Les trois entrées
actuelles de `state/` sont toutes intra-frontière — `pilot-gitconfig` et `pr-origin-epoch` sont
écrits *et* lus par le shell, `auto-pull-stop` est posé à la main par l'opérateur et lu par le
seul Rust (mika#2329). **Aucun fichier de `state/` n'est aujourd'hui écrit par le shell et lu par
le moteur.** Le plan doit donc poser la convention plutôt que l'hériter.

**Et le patron voisin est un piège actif.** `dispatch-lib.sh:5829` écrit
`MIKA_PR_ORIGIN_EPOCH_FILE="${MIKA_HOME:-$HOME/.mika}/state/pr-origin-epoch"` — la seule des six
résolutions shell qui *consulte* `MIKA_HOME`, et elle est **inopérante par construction** :
la variable a été scrubée avant que la ligne s'exécute, donc le `:-` retombe toujours sur
`$HOME/.mika`. C'est du code qui a l'air de gérer le cas et ne le gère pas. Un implémenteur
cherchant un modèle trouvera **celui-là en premier** — c'est le seul qui mentionne `MIKA_HOME` —
le recopiera, et croira le problème résolu. Les cinq autres sites (`:303`, `:346`, `:934`,
`:1176`, `:1260`) écrivent `$HOME/.mika` en dur et sont, eux, honnêtes sur ce qu'ils font.

**Le livrable, et il est petit.** Le shell écrit `$HOME/.mika/state/pilot-egress-down` en dur, à
la manière des cinq sites honnêtes et **jamais** du sixième. Le Rust lit le chemin dérivé de
`global_home_dir`, **déjà câblé sur `TaskDispatcher`** par mika#2329
(`crates/mika-agent/src/server/mod.rs:573`) — rien à propager. Ce qui doit être livré est
**l'invariant qui relie les deux**, écrit aux deux extrémités :

- côté Rust, dans le doc-comment de la fonction de chemin, nommant `scrub_mika_env_vars` et
  disant que le producteur est shell et ne peut pas voir `MIKA_HOME` ;
- côté shell, un commentaire au site d'écriture renvoyant au lecteur Rust et **interdisant
  explicitement** le patron `${MIKA_HOME:-…}` de `:5829`, avec sa raison.

**Et un test qui le tienne, sans quoi il dérive.** L'invariant est de la même famille que
`TTL > seuil` (§3.4.1) : une relation entre deux sites qu'aucun compilateur ne vérifie. Un scan
de source suffit et il est peu coûteux — asserter que le littéral du chemin apparaît à exactement
deux endroits, et que le site shell ne contient pas `MIKA_HOME` sur cette ligne. Sans lui, un
futur éditeur « harmonisant » le shell sur le patron de `:5829` rouvre ce trou en silence, et le
symptôme qu'il produira — des tickets parqués pendant une panne de relais — ne ressemble en rien
à sa cause.

**Alternative écartée** : faire passer le chemin du stamp par une variable d'environnement
dédiée non préfixée `MIKA_` (le patron `PILOT_LOG_DIR` de mika#2249). Elle marcherait, mais
mika#2249 a dû documenter que deux noms distincts pour une même chose est un coût permanent de
compréhension — et il n'y est payé que parce que le répertoire de logs a de bonnes raisons d'être
déplacé. Ici le chemin est fixe, sous le home de l'installation, et personne n'a demandé à le
bouger : une variable créerait la divergence qu'elle prétend gérer.

**Ordre de livraison contraint.** La garde C **seule** satisfait R1 (la sûreté) mais **casse** R5.
A et B seules ne protègent rien. D seule ne protège rien non plus mais évite un mensonge. Les
quatre vont dans le même lot ; si le lot devait être scindé, C ne peut partir ni sans B **ni sans
D** — sans B elle parque les tickets, sans D elle rapporte ses propres refus comme des succès.

### 3.5 Garde D — sans discriminant, le callback classe le refus en SUCCÈS

**Mesure, dans `skills/bundled/self-dev-callback/system_prompt.md`.** Le routage terminal y est
binaire, et les deux branches sont écrites l'une sous l'autre :

- `**On pipeline failure (callback contains "PIPELINE FAILURE:")**`
- `**On success (no "PIPELINE FAILURE:" prefix)**`

Le `RESULT` d'un refus (`dispatch-lib.sh:2659-2668`) ne contient **ni** `PIPELINE FAILURE:`,
**ni** le mot `FAILED`, **ni** le préfixe `STATUS=CANCELLED_` dont le discriminant mika#749
s'occupe en amont, **ni** `error_max_turns`. Le déclencheur secondaire de la classification
pipeline exige en outre un `result` NULL ou vide — or il est renseigné. **Un refus de contenance
tombe donc, littéralement, dans la branche `On success`**, dont la première instruction est
d'aller chercher l'URL de la PR (`gh pr list --head <branch>`) et dont la sortie nominale est de
notifier « claude-pilot completed for {repo}#{issue} » puis de poser la tâche `in_progress` avec
« awaiting QA review ».

**Ce n'est pas une hypothèse sur le modèle : c'est ce que la consigne prescrit.** Le comportement
réel sera au mieux du bricolage hors consigne (la PR n'existe pas), au pire l'annonce d'un succès
pour un pilote qui n'a jamais démarré — *sur le chemin même que le fail-closed rend nominal en cas
de panne*. C'est la classe `assert_grounded` / `milestone-close-claim` que la maison traite le plus
durement, et elle serait ici produite par le substrat, pas par une dérive du modèle.

**Pourquoi ce trou n'a jamais coûté sous mika#2141.** Le refus gitdir est rare — il suppose un
défaut de staging. Le fail-closed d'egress ne crée pas le trou, il le **rend emprunté** : pendant
une panne de relais, chaque dispatch le traverse. Un défaut dormant qu'on met sur le chemin
critique est à traiter dans le lot qui l'y met.

**Le livrable.** Un discriminant de refus de contenance dans `self-dev-callback`, placé **avec**
le discriminant cancel (mika#749) — c'est-à-dire **avant** la classification pipeline, pour la
même raison qu'y est placé le sien :

- **Prédicat** : le `RESULT` contient `CONTAINMENT REFUSAL (exit 78)`. À noter, et c'est une
  contrainte de rédaction : le discriminant cancel est un `starts with`, celui-ci ne peut pas
  l'être — le `RESULT` commence par `Log path: …` (`dispatch-lib.sh:2659`). Un `starts with`
  recopié par analogie ne matcherait jamais et livrerait une garde inerte.
- **Conduite** : **ne pas retry** (un retry immédiat re-frappe un relais toujours mort et brûle
  `pipeline_retry_count` sur une cause qui n'est pas la sienne) ; **ne toucher à aucun label** —
  la reprise est le métier d'`auto_pull`, et retirer `ready` ici casserait R5 par l'autre bout ;
  rendre la tâche terminale avec un motif ; relayer à l'opérateur le texte du refus, qui porte
  déjà la cause et le geste (R2).
- **Vocabulaire** : le motif de `update_task_status` doit être distinct de `operator_cancel` et de
  `signal_cancel`. Un refus de contenance n'est ni l'un ni l'autre, et les trois populations
  doivent rester comptables séparément (doctrine mika#2131, et précédent direct : mika#2249 a dû
  **pré-écrire** son propre discriminant pour ne pas être absorbé par une branche `CANCELLED_BY_*`
  qui aurait fait lire « ne pas retenter » là où il fallait lire « le moteur a disposé »).

**Pourquoi une consigne de prompt ici, alors que R3 refuse l'enforcement par prompt.** La règle
(`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`) porte sur ce qui doit être
**garanti** : la protection (C), l'alerte (§3.3), le filtre (B), la porte (A) sont tous du
substrat déterministe, et aucun n'est confié à un tour LLM. D est d'une autre nature — le callback
*est* un tour LLM par construction, sa classification est déjà prompt-portée de bout en bout, et
ce lot ne peut pas convertir ce chemin en substrat sans réécrire `self-dev-callback` en entier.
Ce qui est livré ici est donc l'ajout d'une **branche manquante à une machine à états qui en a
déjà trois**, pas une nouvelle dépendance à l'obéissance du modèle : le défaut mesuré n'est pas
que le modèle désobéit, c'est que la consigne prescrit aujourd'hui la mauvaise branche.
Durcir davantage — une garde EndTurn sur l'annonce d'un succès sans PR — est **hors périmètre** et
nommé comme tel en §6 : son lexique croiserait le trafic nominal de tous les callbacks sains.

### 3.6 Préalable 3 — remise en marche outillée et runbook (R6)

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
7. **le stamp est encore frais quand Phase 2 regarde** (§3.4.1) — test Rust sur les **valeurs par
   défaut** : à un âge de label égal au seuil de stuck-ready, un stamp du même âge est jugé frais
   et B rend `Skip`. C'est l'assertion qui tient l'invariant `TTL > seuil` ; sans elle, les deux
   constantes peuvent dériver l'une par rapport à l'autre sans qu'aucun test ne rougisse
   (mika#2362). Elle doit **échouer contre un TTL de 600 s**, ce qui est le contrôle négatif de la
   correction elle-même.
7bis. **les deux extrémités du stamp désignent le même fichier** (§3.4.2) — scan de source : le
   littéral `state/pilot-egress-down` n'apparaît qu'aux deux sites prévus, et la ligne shell qui
   l'écrit ne contient pas `MIKA_HOME`. C'est l'assertion qui tient la seconde relation
   inter-sites du lot, de la même famille que l'assertion 7. **Un test comportemental ne peut pas
   l'attraper** : il poserait le même home des deux côtés, la divergence ne rendrait aucune
   décision fausse, et la garde B deviendrait inatteignable en silence.

8. **un refus n'est pas rapporté comme un succès** (§3.5) — le `RESULT` produit par l'assertion 1
   est passé au discriminant de `self-dev-callback` : il route vers la branche de refus de
   contenance, et **non** vers `On success`. Vérifiable sans tour LLM en assertant que le texte
   porte le marqueur `CONTAINMENT REFUSAL (exit 78)` que le discriminant cherche — l'obéissance du
   modèle n'est pas testable ici, la présence du marqueur qu'il doit lire l'est.

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
La présence d'`egress_relay_down` prouve que B mord. Son absence pendant une panne a **deux
causes distinctes et il faut les départager dans cet ordre**, parce qu'elles produisent un
symptôme rigoureusement identique — une garde qui paraît absente alors qu'elle est seulement
inatteignable :

1. **Le stamp n'est pas là où le moteur regarde** (§3.4.2). Question décidable en une commande :
   `ls -la "$HOME/.mika/state/pilot-egress-down"` **et** la même sous `$MIKA_HOME` si la variable
   est posée sur le service. Deux chemins distincts ⇒ c'est la divergence de §3.4.2, et le remède
   est le chemin, pas le réglage.
2. **Le stamp est systématiquement périmé quand Phase 2 regarde** (§3.4.1). Lire
   `MIKA_PILOT_EGRESS_DOWN_TTL_SECS` contre `MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS` : un TTL
   retombé sous le seuil restaure exactement le tableau de §3.4.1.

L'ordre est imposé par le coût de l'erreur : chercher un défaut de TTL sur une divergence de
chemin conduit à rallonger le TTL, ce qui ne répare rien et **ajoute** de la latence de reprise.
Dans tous les cas, **réparer la lecture, ne pas allonger le budget de re-drive**, qui masquerait
le symptôme sans toucher la cause.

**(f) Halte — un refus annoncé comme un succès.** Après un refus avéré (une ligne `CONTAINMENT
REFUSAL` dans un `RESULT`), vérifier qu'aucune notification « claude-pilot completed » ne porte le
même ticket et qu'aucune tâche n'est restée `in_progress` sur « awaiting QA review » sans PR. Une
occurrence signifie que le discriminant de §3.5 n'est pas atteint — vérifier **d'abord** qu'il est
écrit en `contains` et non en `starts with` (le `RESULT` commence par `Log path:`), avant de
soupçonner le modèle. **Ne pas répondre à cette halte par une garde EndTurn** : voir §6, son
lexique croiserait le trafic nominal.

**(e) Contrôle négatif du déploiement.** L'absence de refus ne prouve pas que la garde est en
vigueur : elle est identique à l'absence de panne. Pour établir le déploiement, exercer
`scripts/canary-pilot-containment --restart-relay` et vérifier la reprise — et se rappeler qu'un
binaire antérieur au correctif produit exactement le même silence (classe mika#2340).

---

## 6. Hors périmètre, délibérément

- **Le relais wedgé** (accepte `connect()`, ne sert plus). Nommé en §3.6, doté d'un geste
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
- **Une garde EndTurn contre l'annonce d'un succès sans PR.** Elle fermerait la classe de §3.5 au
  substrat plutôt qu'au prompt, et c'est la bonne direction à terme. Elle est écartée **sur
  mesure**, pas par prudence : son lexique — « completed », « PR », « awaiting QA » — est le
  vocabulaire nominal de *tous* les callbacks sains, dont le régime est précisément d'annoncer une
  PR. Le taux de faux positifs porterait sur la population que la garde doit épargner, et un faux
  positif y casse un tour de callback légitime. **Ticket de suivi**, préalable écrit : un
  discriminant qui ne soit pas lexical (l'existence effective de la PR, lue et non dite).
- **La conversion de `self-dev-callback` en classification de substrat.** La garde D ajoute une
  branche à une machine à états prompt-portée ; elle ne change pas sa nature. Réécrire ce chemin
  est un lot à soi seul, dont le blast radius est tous les dispatches.

---

## Definition of Done

- [ ] `_ensure_pilot_egress_proxy` pose un motif structuré ; `_run_pilot_sandboxed` refuse
      (`return 78`) au lieu de retomber en Phase 2a, **avant** `_stage_pilot_gh_token`.
- [ ] Le `RESULT` de refus nomme la cause et le geste de remise en marche, **et sa queue fixe
      ne prescrit plus de réparer le worktree** sur une cause d'egress (§3.2) — le texte de refus
      est entièrement porté par le motif.
- [ ] `mika notify --channel telegram --severity escalate` est émis au premier refus d'un épisode,
      en `|| true`, et une notification `info` annonce la reprise.
- [ ] **Praticabilité du canal établie au déploiement** (§3.3.1) : l'agent `mika` porte un
      `chat_id` non nul en `customer_config` et une notification de test est reçue. Sans cette
      case, l'AC1 livre un appel émis et non une escalade lue.
- [ ] `~/.mika/state/pilot-egress-down` est écrit/retiré par la garde C, avec péremption
      `MIKA_PILOT_EGRESS_DOWN_TTL_SECS` (défaut **1800**).
- [ ] **L'invariant `TTL > seuil de stuck-ready` est écrit dans le doc-comment de la constante**,
      en nommant `STUCK_READY_THRESHOLD_ENV`, et tenu par l'assertion 7 de §5.1 (§3.4.1).
- [ ] **Les deux extrémités du stamp désignent le même fichier** (§3.4.2) : le shell écrit
      `$HOME/.mika/state/pilot-egress-down` **en dur**, jamais via `${MIKA_HOME:-…}` ; le Rust lit
      depuis `global_home_dir` ; l'invariant est écrit **aux deux sites** (le doc-comment Rust
      nommant `scrub_mika_env_vars`, le commentaire shell interdisant le patron de
      `dispatch-lib.sh:5829` avec sa raison) et tenu par l'assertion 7bis de §5.1.
- [ ] Garde B (`auto_pull` Phase 2, filtre `egress_relay_down`, verdict `Skip`) et garde A
      (`ready_label_handler`, porte `egress_relay_down`, refus `Handled`) livrées dans le même lot.
- [ ] **Garde D** — `self-dev-callback` porte un discriminant de refus de contenance (prédicat
      `contains`, jamais `starts with`), placé avec le discriminant cancel mika#749, sans retry,
      sans toucher aux labels, avec un motif distinct d'`operator_cancel` et de `signal_cancel`.
- [ ] `scripts/canary-pilot-containment --restart-relay` livré.
- [ ] `docs/operator/pilot-egress-relay.md` livré, avec la note de désambiguïsation « egress
      pilote ≠ egress recherche ».
- [ ] Les huit assertions de §5.1 passent ; les assertions 1 et 3 échouent contre `HEAD`, et
      l'assertion 7 échoue contre un TTL de 600 s.
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
(`operator-review`) ni n'a consommé de budget de re-drive du fait de la panne. Vérifiable en
**trois** moitiés, et aucune ne suffit sans les deux autres : *le verdict* de la garde B par
§5.1 assertion 6 (`Skip`, jamais `SkipAndResetBudget`) ; *son atteignabilité dans le temps* par
l'assertion 7 (le stamp est encore frais quand Phase 2 regarde, §3.4.1) ; *son atteignabilité
dans l'espace* par l'assertion 7bis (les deux gardes désignent le même fichier, §3.4.2). Une
garde B correcte dont le stamp est systématiquement périmé — ou cherché ailleurs qu'il n'est
écrit — rend AC5 **vert en test et faux en production**, et les deux défaillances produisent le
même symptôme qu'une garde absente.

**AC8 — Un refus n'est pas rapporté comme un succès.** Le `RESULT` d'un refus de contenance est
routé par `self-dev-callback` vers une branche de refus, jamais vers `On success` ; aucune
notification n'annonce une PR ou une complétion. Vérifiable : §5.1 assertion 8. *Dérivé, non
transcrit : ce critère ne figure pas dans le commentaire opérateur du 2026-09-20, il sort de la
mesure de §3.5 — le fail-closed met sur le chemin nominal d'une panne un trou de classification
jusqu'ici dormant.*

**AC6 — Aucune régression de contenance.** Les quatre invariants de §5.2 restent testés et verts
après migration.

**AC7 — Pas d'échappatoire.** Aucune variable d'environnement ne lève le refus (option 2 écartée
par décision opérateur). Vérifiable : absence de toute lecture d'environnement dans la branche de
refus de la garde C.
