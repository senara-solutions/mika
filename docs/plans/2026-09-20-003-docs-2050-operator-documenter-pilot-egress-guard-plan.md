# mika#2050 — Documenter `pilot_egress_guard.unreachable` dans la série des signaux opérateur

- **Ticket :** senara-solutions/mika#2050
- **Type :** docs (une entrée dans `CLAUDE.md`, aucun changement de substrat)
- **Branche :** `docs/2050/operator-documenter-pilot-egress-guard`
- **Date :** 2026-09-20

---

## Résumé

Le ticket demande une entrée dans la série « Signal A … Signal P » de `CLAUDE.md`
pour le jeton `pilot_egress_guard.unreachable`, et nomme lui-même sa seule
inconnue : *« à vérifier : quel fichier capte réellement le stderr de
`dispatch-lib` »*.

Cette vérification a été faite avant toute rédaction. **Elle rend une réponse
précise, et elle invalide trois choses que l'entrée « évidente » aurait
affirmées.** Le livrable principal de ce plan est donc autant la trajectoire
établie que l'entrée qui en découle : écrite au modèle de ses voisines, l'entrée
aurait nommé `$MIKA_SPIRIT_LOG_FILE`, et elle aurait été fausse.

---

## Ce que la vérification a établi

Chaîne complète, lue dans le code à HEAD (`467279bd`), du site d'émission
jusqu'au fichier sur disque.

| # | Fait | Source |
|---|---|---|
| **F1** | Site d'émission **unique** du jeton. | `skills/bundled/_shared/dispatch-lib.sh:531`, dans `_ensure_pilot_egress_proxy` (défini l. 491) |
| **F2** | **Un seul** appelant de cette garde. | `_run_pilot_sandboxed`, l. 973 |
| **F3a** | `_run_pilot_sandboxed` est invoquée sous `2>"$STDERR_FILE"` (un `mktemp`), puis le contenu est **scrubé et persisté** vers `$_PILOT_LOG_DIR/<LOG_ID>.stderr` si non vide. ⇒ **capté** | `_run_claude_pilot`, l. 2579 puis 2583–2587 |
| **F3b** | Second appelant : `_launch_revise_pilot` redirige vers `$(mktemp /tmp/revise-stderr-XXXXXX)`, **supprimé** l. 5252 sans copie persistante. ⇒ **perdu** | l. 5244–5252 |
| **F4** | `_PILOT_LOG_DIR = ${PILOT_LOG_DIR:-/var/log/claude-pilot}`, et le fichier est **par task-id**. | l. 247 |
| **F5** | Ce sink est déjà la surface forensique établie : `dispatch-lib` le relit elle-même pour en extraire `[policy:deny]`. | l. 3558 et 3751 (mika#1097) |
| **F6** | Le repli « binaire absent » **ne porte pas le jeton** — autre message, même conséquence. | l. 493 vs l. 531 |
| **F7** | Le log du proxy est `${MIKA_PILOT_EGRESS_LOG_DIR:-/var/log/mika}/pilot-egress-proxy.log`, avec repli `/tmp/mika-pilot-egress-proxy.log` si le répertoire n'est pas inscriptible. | l. 505–510 |

**Réponse à l'inconnue du ticket :** le stderr de `dispatch-lib` émis *pendant la
session pilote* atterrit dans **`/var/log/claude-pilot/<task-id>.stderr`**. Ni
`server.log`, ni `$MIKA_SPIRIT_LOG_FILE`.

### Pourquoi la vérification était nécessaire, et pas une formalité

Les dix-huit entrées existantes greppent toutes un fichier unique, et les deux
qui concernent `dispatch-lib` nomment `server.log` (Signal M) et
`$MIKA_SPIRIT_LOG_FILE` (Signal Q). Recopier ce modèle produisait une commande
qui ne rend **jamais** la ligne. C'est exactement la dérive que le Signal O a dû
faire corriger par mika#2069 — une instruction de sink fausse qui a vécu dans
`CLAUDE.md` en rendant « un jeu de données plus petit, plausible, faux ».

---

## Trois rectifications que l'entrée doit porter

### R1 — Le grep du ticket est incomplet par construction (F6)

Il existe **deux** replis vers le mode dégradé, et **un seul** porte le jeton :

| repli | message | jeton |
|---|---|---|
| le proxy ne bind pas dans les 3 s | `pilot_egress_guard.unreachable …` | **oui** |
| le binaire est absent (`! -x`) | `mika-pilot-egress-proxy not found at … (falling back to fs-only)` | **non** |

Le second est le cas explicitement prévu par le commentaire du code pour la
fenêtre de déploiement. Conséquence directe : **« zéro occurrence du jeton » ne
prouve pas que la coupure réseau est active.** Un opérateur qui applique la
commande du ticket telle quelle lit un régime nominal sur une flotte dont le
binaire n'est pas déployé.

La chaîne commune aux deux lignes est `falling back to fs-only`. L'entrée
grepperait donc ce prédicat-là, qui couvre la population entière, en nommant
le jeton comme le discriminant entre les deux causes.

### R2 — Un chemin ne capte rien, et une absence n'y prouve rien (F3b)

Sur le chemin `_launch_revise_pilot` — c'est-à-dire la branche ITERATE du
grooming autonome — la ligne est écrite dans un `mktemp` sous `/tmp` qui est
supprimé quelques lignes plus bas. Aucune copie. L'entrée doit le dire : le
régime « zéro occurrence » vaut pour le chemin dispatch, **pas** pour le chemin
revise, où le silence est structurel.

Réparer ce trou est ~2 lignes (persister comme le fait `_run_claude_pilot`),
mais c'est un changement de substrat sur le chemin de dispatch, hors du
périmètre « une entrée dans `CLAUDE.md` ». → **ticket de suivi**, nommé dans
la section Suivi.

### R3 — Corollaire avéré : le Signal M existant est faux

`_check_pilot_force_push` est appelé au top-level de `dispatch_claude_pilot`
(l. 6924), **après** `_run_claude_pilot` — donc **hors** de la redirection
`2>"$STDERR_FILE"`, qui n'existe qu'autour de la l. 2579. Son stderr est celui
que le handler a hérité de `spawn_long_running_exec`
(`crates/mika-agent/src/skills/executor.rs:3491–3493` : `stderr(Stdio::piped())`),
dont le handle **n'est lu que dans la branche `if !status.success()`**
(l. 3644–3654). Sur un dispatch qui réussit, le pipe est droppé sans lecture.

⇒ `grep pilot_push_guard server.log` (Signal M) ne rend rien. La ligne
`pilot_push_guard.clean`, annoncée « expected on every dispatch », est
structurellement absente — et surtout `pilot_push_guard.violation`, annoncée
« should never appear », **ne peut pas apparaître**. Un signal de sécurité qui
lit « rien à signaler » quel que soit l'état.

C'est la même classe que le ticket, sur la ligne d'à côté, découverte par la
vérification que le ticket prescrit. Le corriger *correctement* suppose de
décider où rediriger ces lignes — substrat, donc hors périmètre. Le plan
retient : **une halte nommée dans l'entrée S** (pour qu'un opérateur ne
transporte pas la commande de M par analogie) + **ticket de suivi**. Ne rien
dire laisserait sciemment une instruction fausse que ce travail vient de
démontrer fausse.

---

## Le livrable

Une entrée **Signal S** dans `CLAUDE.md` § *Post-restart safety check* (A–R sont
pris ; S est la première lettre libre), au format exact des voisines : commande,
régime permanent, seuil d'anomalie, remède.

Sa substance :

- **Commande** — grep de `falling back to fs-only` sur
  `${PILOT_LOG_DIR:-/var/log/claude-pilot}/*.stderr` (glob : un fichier par
  task-id, à la différence de toutes les entrées existantes).
- **Régime permanent** — zéro occurrence. Pas un compteur qui tolère un bruit
  de fond.
- **Anomalie** — toute occurrence : un pilote est parti sans coupure réseau.
  Le jeton `pilot_egress_guard.unreachable` discrimine « le proxy n'a pas
  bindé » de « le binaire est absent », et les deux appellent des remèdes
  différents.
- **Remède** — inspecter
  `${MIKA_PILOT_EGRESS_LOG_DIR:-/var/log/mika}/pilot-egress-proxy.log` (repli
  `/tmp/mika-pilot-egress-proxy.log` si le répertoire n'est pas inscriptible —
  nommer les deux, sinon l'opérateur cherche un fichier absent).
- **Halte 1** — le grep est vide *et* `PILOT_LOG_DIR` / `MIKA_PILOT_LOG_DIR`
  divergent : le glob pointe ailleurs qu'où `dispatch-lib` écrit. Établir la
  configuration avant de conclure (la divergence de ces deux variables est déjà
  documentée sous mika#2249).
- **Halte 2** — ne pas transporter cette commande vers le Signal M par
  analogie : ces lignes-là ne sont captées par aucun fichier (R3).
- **Limite** — le chemin revise ne capte rien (R2).

---

## Périmètre

**Dans le périmètre :** une entrée dans `CLAUDE.md`. Rien d'autre.

**Hors périmètre, délibérément :** la persistance du stderr du revise pilot
(R2), la trajectoire des lignes `pilot_push_guard` (R3), et l'unification des
deux replis sous un jeton `pilot_egress_guard.*` unique — qui serait la
correction de fond de R1 et relève de la doctrine « les noms d'événement sont un
format de fil » (mika#2131, mika#2323). Les trois sont des changements de
substrat ; ce ticket documente.

---

## Ce que ce travail n'achète pas

Aucun compteur, aucun événement nouveau, aucune ligne `audit_events`. Le seul
instrument est le grep ci-dessus, et **son silence ne prouve rien tant que
personne ne l'exécute** — c'est la limite que le ticket énonce lui-même (« un
signal que personne ne cherche n'est pas un progrès sur un silence »), et cette
entrée ne la lève pas : elle rend le signal cherchable, pas cherché. La ligne
reste absente du chemin revise (R2) et la population « binaire absent » reste
sans jeton stable (R1).

---

## Vérification

1. Les chemins, variables et numéros de ligne cités dans l'entrée existent à
   HEAD — re-vérifiés par grep au moment de la rédaction, pas recopiés du plan.
2. Les deux messages de repli portent bien tous deux la chaîne
   `falling back to fs-only` (`grep -c` sur `dispatch-lib.sh` ⇒ 2).
3. La lettre `S` est libre dans la série.
4. L'entrée est placée dans § *Post-restart safety check*, au format des
   voisines (commande / régime / anomalie).
5. Aucun fichier hors `CLAUDE.md` et `docs/plans/` n'est modifié
   (`git diff --stat`).
6. `make verify-bundled-skills` n'est pas concerné (aucun bundle touché) ;
   la sync `docs/` n'est pas concernée (`CLAUDE.md` n'est pas sous `docs/`).

---

## Definition of Done

- [ ] L'entrée **Signal S** est ajoutée à `CLAUDE.md` § *Post-restart safety
      check*, au format des entrées voisines.
- [ ] Elle nomme le sink réel `${PILOT_LOG_DIR:-/var/log/claude-pilot}/*.stderr`
      et **jamais** `server.log` ni `$MIKA_SPIRIT_LOG_FILE`.
- [ ] Elle couvre les **deux** motifs de repli (R1) et nomme le jeton comme
      discriminant, pas comme prédicat de la population.
- [ ] Elle nomme la limite du chemin revise (R2) et la halte sur le Signal M (R3).
- [ ] Elle nomme le log du proxy **et** son repli `/tmp` comme remède.
- [ ] Aucun fichier de substrat n'est modifié.
- [ ] Les deux tickets de suivi sont nommés dans le corps de la PR.

## Acceptance criteria

Dérivés du corps du ticket (qui ne porte pas de section `## Acceptance
criteria`) et des rectifications établies par la vérification.

- **AC1** — `CLAUDE.md` contient une entrée pour `pilot_egress_guard.unreachable`
  dans la série des signaux opérateur, portant les trois champs canoniques :
  une commande de `grep`, un attendu en régime permanent, un seuil d'anomalie.
- **AC2** — Le régime permanent déclaré est **zéro occurrence**, explicitement
  présenté comme n'admettant pas de bruit de fond.
- **AC3** — L'anomalie déclarée est **toute occurrence**, avec pour remède
  l'inspection du log du proxy, dont le chemin par défaut **et** le repli `/tmp`
  sont nommés.
- **AC4** — La commande porte sur
  `${PILOT_LOG_DIR:-/var/log/claude-pilot}/*.stderr`. Une entrée nommant
  `server.log` ou `$MIKA_SPIRIT_LOG_FILE` échoue cet AC : ces fichiers ne
  contiennent jamais la ligne (F1–F5).
- **AC5** — L'entrée couvre la population entière des replis vers `fs-only`,
  pas le seul jeton, et dit explicitement que l'absence du jeton ne prouve pas
  que la coupure réseau est active (R1 / F6).
- **AC6** — L'entrée nomme la limite du chemin `_launch_revise_pilot`, où
  aucune occurrence n'est captée (R2 / F3b).
- **AC7** — L'entrée porte une halte interdisant de transporter sa commande
  vers le Signal M, dont les lignes ne sont captées par aucun fichier (R3 / F6).
- **AC8** — Aucun fichier hors `CLAUDE.md` et `docs/plans/` n'est modifié :
  le ticket est documentaire, et les trois défauts de substrat qu'il a permis
  d'établir partent en tickets de suivi.

---

## Suivi (à ouvrir, non traité ici)

1. **Le stderr du revise pilot n'est persisté nulle part** — `_launch_revise_pilot`
   écrit dans un `mktemp` sous `/tmp` puis le supprime, là où `_run_claude_pilot`
   persiste vers `$_PILOT_LOG_DIR/<LOG_ID>.stderr`. Tout diagnostic sur la
   branche ITERATE du grooming est structurellement aveugle.
2. **Signal M est un signal de sécurité qui ne peut pas firer** —
   `pilot_push_guard.{clean,violation}` est émis hors de toute redirection, dans
   un pipe que `spawn_long_running_exec` ne lit que sur échec. Même classe que
   mika#2050, même fichier ; à traiter au substrat (rediriger vers le sink
   forensique) plutôt qu'en corrigeant la commande documentée.
3. **Les deux replis egress devraient partager un jeton** — la population « le
   pilote est parti sans coupure réseau » n'a pas de nom stable ; seule sa
   moitié « proxy mort » en a un. Doctrine « format de fil » (mika#2131,
   mika#2323).
