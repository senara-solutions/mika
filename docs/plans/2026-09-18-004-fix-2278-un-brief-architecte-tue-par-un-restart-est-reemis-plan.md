# mika#2278 — un brief architecte tué par un restart est ré-émis, pas attendu indéfiniment

- **Issue:** senara-solutions/mika#2278
- **Type:** fix
- **Priorité:** p2 (ralentit la boucle)
- **Date:** 2026-09-18

---

## Problem frame

Un groom en vol pendant un restart `mika-spirit` voit son appel
`mika ask --agent mika-arch` tué, et la passe d'architecte est perdue. Sur le
flux opérateur, le pilote reste pendu au prompt idle (1 h 50 mesurées sur le
groom de #2276, jusqu'à un nudge manuel). Sur le flux autonome, la passe meurt
en `PIPELINE_INCOMPLETE` sans que rien ne la reprenne.

**Deux mesures déplacent le diagnostic du ticket, et il faut les poser avant de
choisir un remède — parce qu'elles invalident chacune une des deux pistes.**

### M1 — la piste (a) est très largement déjà fermée, par deux tickets

Le ticket demande que « `mika ask --session-id <X>` RETOURNE une erreur/vide
quand le tour de la session a été tué (pas laisser l'appelant pendre) ».

Le code fait déjà exactement cela, sur les deux moitiés :

- `crates/mika-cli/src/remote_ask.rs` classe toute panne de transport
  (`TransportFailure::classify`), tente une récupération par `context_id`
  quand la requête est partie, et **échoue avec un message qui nomme ce qu'est
  devenu le travail** — cinq formulations distinctes selon la variante
  `Recovery` (mika#2036).
- `crates/mika-cli/src/commands/ask.rs` : une réponse illisible est une
  **erreur**, plus un `content` absent à exit 0 (mika#2270).
- Et la cause immédiate mesurée — « timeouté à 300 s côté CLI » — a été fermée
  par **mika#2297** : `A2aClient::DEFAULT_TIMEOUT` est passé à 600 s avec un
  plancher `client >= MIKA_AGENT_TOTAL_TIMEOUT_SECS` (`crates/mika-a2a/src/client.rs:24`),
  précisément pour qu'un client n'abandonne plus une génération que le moteur a
  encore le droit de finir.

`mika ask` ne pend pas et ne ment pas. **Ce qui manquait n'est pas que le CLI
le dise — c'est que l'appelant l'écoute.**

Reste un résidu réel, à nommer plutôt qu'à corriger ici : quand le tour est
encore vivant au moment de la panne, le message est
`Recovery::StillRunning → « … Retry. »`, qui est **vrai à l'instant où il est
écrit**. Le CLI ne peut pas prédire un restart à venir. Exiger de lui qu'il
réponde « mort » serait lui demander de deviner.

### M2 — la piste (b) nomme un fichier hors de ce dépôt, et une forme que la doctrine condamne

`.claude/commands/mika-groom-ticket.md` n'est **pas** suivi par git ici :
`git ls-files .claude/commands/` ne rend que quatre fichiers
(`mika.md`, `mika-issue.md`, `mika-issues.md`, `mika-doc-audit.md`). Le reste
est *seedé* dans le worktree depuis le méta-dépôt par
`_seed_worktree_slash_commands()` (mika#1415) et masqué via `info/exclude`. Un
correctif écrit là ne serait pas livrable par cette PR.

Et même livrable ailleurs, ce serait de l'enforcement par prompt sur le
substrat de boucle, que
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` refuse
sur mesure : mika#2120 a compté **9 récurrences sous enforcement par prompt
contre 0 quand le fait est posé par du code**.

### Le défaut qui reste, et il est dans ce dépôt

Le chemin de la boucle autonome est `_arch_ask`
(`skills/bundled/_shared/dispatch-lib.sh:4651`), appelé en quatre points de
`_iterate_groom_loop` (lignes 5448, 5468, 5511, 5560). Il porte **deux** défauts
composés :

1. **Aucun retry.** Chaque site fait `|| { _groom_warn "…"; return 1; }`. Une
   panne de transport — la classe la plus manifestement transitoire qui soit —
   coûte la passe entière, le slot de dispatch, et un point du budget de
   re-drive (mika#2020 : trois tours abandonnent un ticket sain).

2. **`2>/dev/null` sur les quatre sites.** Le message de `mika ask` part sur
   stderr (`crates/mika-cli/src/main.rs:332,355`) et il est **jeté**. La boucle
   ne voit qu'un code de sortie `1`, identique pour « le serveur redémarre » et
   pour « ta session appartient à un autre agent ». *Le seul canal qui porte la
   distinction est fermé par l'appelant.*

Le second est ce qui rend le premier non trivial : on ne peut pas réessayer à
bon escient tant qu'on ne peut pas distinguer transitoire de définitif. C'est
aussi le travail que le `CLAUDE.md` annonce déjà comme en aval :
« le retry d'`_arch_ask` (mika#2331, en aval — AC1/AC2 lui rendent sa
précondition) ».

---

## Décisions de conception

### D1 — la classe vient de la variante, jamais d'une sous-chaîne du message

Discriminer en shell par `grep` sur « Retry. » ou « still working » ferait d'une
phrase écrite pour un humain un **format de fil**. La maison a déjà tranché deux
fois : mika#2179 (« les classes d'erreur viennent de la *variante* `LlmError`,
via `downcast_ref`, jamais d'un match de sous-chaîne sur le message rendu ») et
mika#2291 (« le déclencheur du repli est le **statut 400**, jamais une
sous-chaîne de la `description` »).

Donc : `mika ask` **rend un code de sortie dédié** quand l'échec est de classe
transport. Le texte du message ne change pas d'un octet — il reste pour
l'humain, il ne devient pas un contrat.

### D2 — la ligne de partage est *transport* vs *contrat*, et elle inclut « serveur injoignable »

| Situation | Classe | Pourquoi |
|---|---|---|
| serveur injoignable (`request_was_sent() == false`) | **retryable** | **c'est exactement le cas du ticket** : pendant un restart le port ne répond pas |
| `Recovery::StillRunning` | retryable | le serveur travaille ; revenir est la bonne réponse |
| `Recovery::NoTask` | retryable | requête non atterrie, ou refusée au lock (agent occupé) |
| `Recovery::Ended` | retryable | l'état qu'écrit `startup_recovery` après un restart est précisément `failed` |
| `Recovery::Unavailable` | retryable | on ne sait pas ; voir l'asymétrie ci-dessous |
| JSON-RPC invalide, sérialisation, transition d'état invalide | **définitif** | un contrat cassé ne se répare pas en réessayant |
| session d'un autre agent, `--task-id` inconnu, message vide | définitif | erreurs d'usage, déjà refusées avant tout envoi |

**L'asymétrie qui décide les cas douteux.** Un faux « retryable » coûte un tour
d'architecte payé deux fois (quelques minutes, quelques centimes). Un faux
« définitif » coûte la passe, le slot, et un point du budget de re-drive — trois
tours et le ticket sain est abandonné en `operator-review` (mika#2020). Le coût
n'est pas du même ordre, donc la classe transport penche entièrement du côté
retryable — **bornée à un seul retry**, comme tout budget de la maison.

Corollaire : `startup_recovery` (`task_engine/engine.rs:402-433`) marque déjà
`failed` toute tâche `in_progress` non-`manual` au démarrage, et une tâche A2A a
`trigger_type = 'a2a'` — donc un tour tué par restart **est** transitionné. Ce
plan n'a pas à y toucher : il a à en faire quelque chose.

### D3 — code `75` (`EX_TEMPFAIL`), et rien d'autre ne bouge

`75` est la valeur `sysexits.h` pour « échec temporaire, réessayez ». Elle ne
collisionne avec rien : le CLI n'émet aujourd'hui que `0` et `1` (plus les codes
de clap). **Aucun consommateur existant ne régresse** : les autres appelants de
`mika ask` dans `skills/bundled/**` sont tous des `--task-complete`
(`address-pr-comments`, `deploy-mika`, `resolve-pr-conflicts`), un chemin qui
retourne **avant** l'envoi A2A et ne peut donc jamais produire `75`.

Sur le chemin `--remote`, la même classification s'applique : c'est le même
`send_message_to_agent`.

### D4 — fail-safe dans le sens de la maison : un code inconnu n'est jamais retryable

`_arch_ask_with_retry` ne réessaie que sur `75`, jamais sur « tout sauf 0 ». Un
code qu'on ne sait pas lire est traité comme définitif. L'inverse ferait d'un
futur mode d'échec non prévu une boucle de réessais silencieuse.

### D5 — la session, et pourquoi le premier appel est le cas facile

Le **premier** appel n'a pas encore de `session_id` (il est extrait de la
réponse). Son retry repart donc sur une session neuve — ce qui est très
exactement le geste manuel qui a débloqué #2276 (« après nudge : re-émission
avec nouvelle session → complété normalement »).

Les appels 2 à 4 passent `$session_id` et le retry le conserve : l'architecte
voit son propre tour précédent, ce que le contrat de continuité de
`mika-arch-second-review` demande.

**Coût nommé :** si le tour tué avait déjà persisté son message user, le retry
sur la même session fait voir le même prompt deux fois à l'architecte.
Inoffensif (il répond à la dernière occurrence) mais réel, et c'est la raison
pour laquelle le budget est de **un** retry et non de trois.

### D6 — un délai avant le retry, sinon le retry tombe dans le même trou

Un restart n'est pas instantané. Réessayer dans la seconde retomberait sur le
même port mort et brûlerait le budget pour rien. Délai par défaut **30 s**,
trois paliers habituels (absent/vide → défaut ; illisible, `0` ou négatif →
défaut + WARN). Borné aussi par le haut : un délai absurde immobiliserait le
slot de groom plus longtemps que la panne qu'il absorbe.

### D7 — stderr est capturé, plus jeté

Les quatre `2>/dev/null` deviennent une capture vers un fichier temporaire dont
le contenu part dans le WARN. Sans cela, l'opérateur qui lit
`first-pass _arch_ask failed` n'apprend toujours rien, et la sonde
post-déploiement ci-dessous est inexécutable. C'est la moitié *observabilité*
du correctif, et elle a sa propre valeur même si le retry n'aboutit pas.

---

## Requirements

- **R1** — `mika ask` sort en `75` lorsque l'échec est de classe transport
  (D2), sur le chemin spirit comme sur `--remote`. Le texte des messages est
  inchangé.
- **R2** — Aucun autre code de sortie ne change. `0` en succès, `1` pour tout
  échec de contrat.
- **R3** — `_arch_ask` réessaie **une** fois, et seulement sur `75`.
- **R4** — Le retry attend `MIKA_ARCH_ASK_RETRY_DELAY_SECS` (défaut 30) avant de
  repartir.
- **R5** — Les quatre sites d'appel capturent stderr et le font apparaître dans
  le WARN d'échec.
- **R6** — Le retry et son issue sont journalisés, avec un nom d'événement dont
  ce site est le seul écrivain.
- **R7** — Le budget est désarmable sans redéploiement du binaire
  (`MIKA_ARCH_ASK_RETRY=0`).

---

## Implementation steps

### Étape 1 — `crates/mika-cli/src/remote_ask.rs` : typer la classe

1. Introduire un type porteur de la classe, par exemple
   `AskFailure { message: String, retryable: bool }`, ou un marqueur sur
   l'erreur `anyhow` (`.context`-free ; un type nommé, pas une sous-chaîne).
2. Le poser à l'unique endroit qui connaît déjà la distinction : le bras
   `Err(A2aError::ClientError(e))` de `send_message_to_agent`, où `failure` et
   `recovery` sont tous deux en main. `retryable = true` pour ce bras ;
   `false` pour `InvalidJsonRpc`, `SerializationError`,
   `InvalidStateTransition`.
3. Le bras de validation d'état terminal (`Submitted | Working | Unknown`
   rendus par un `message/send` synchrone) est **retryable** : c'est un état
   que le serveur ne devrait pas rendre là, donc une anomalie transitoire.

### Étape 2 — `crates/mika-cli/src/commands/ask.rs` + `main.rs` : propager le code

4. `wrap_send_error` préserve la classe (elle est aujourd'hui aplatie dans un
   `anyhow!`). Le mask-through de mika#1985 doit rester intact — le test
   `test_wrap_send_error_preserves_underlying_a2a_error_chain` est la garde.
5. Dans `main.rs`, les deux sites `eprintln!("Error: …"); std::process::exit(1)`
   sortent `75` quand la classe est transport. Le texte imprimé ne change pas.

### Étape 3 — `skills/bundled/_shared/dispatch-lib.sh` : le retry borné

6. Ajouter `_arch_ask_with_retry()` enveloppant `_arch_ask` : capture stdout,
   capture stderr dans un tmpfile, lit le code de sortie ; sur `75` et si
   `MIKA_ARCH_ASK_RETRY != 0`, émet `arch_ask_retry` (INFO, champs `skill`,
   `attempt`, `delay_secs`), dort le délai, rejoue une fois avec les mêmes
   arguments ; sinon propage le code tel quel.
7. Remplacer les quatre appels de `_iterate_groom_loop` (5448, 5468, 5511,
   5560) par `_arch_ask_with_retry`, et supprimer les `2>/dev/null` au profit
   de la capture (R5).
8. Sur échec définitif après retry : `arch_ask_retry_exhausted` (WARN), et le
   WARN existant porte désormais la dernière ligne de stderr.
9. Lecteurs d'environnement en trois paliers, à côté des lecteurs voisins du
   fichier.

### Étape 4 — tests

10. `skills/bundled/_shared/test-dispatch-lib.sh` (suite existante, câblée en CI
    via `make test-dispatch-lib`) : un `mika` factice sur le `PATH` rendant
    `75` puis `0` ; asserter exactement **deux** invocations et un succès final.
11. Contrôle négatif — c'est celui qui porte la preuve : un factice rendant `1`
    (contrat) doit produire **une seule** invocation. Sans lui, un
    `_arch_ask_with_retry` qui réessaierait sur n'importe quel code non nul
    passerait le test positif.
12. Contrôle négatif de désarmement : `MIKA_ARCH_ASK_RETRY=0` + code `75` → une
    seule invocation.
13. Côté Rust : tests unitaires sur la classification (une variante `Recovery`
    par cas, plus `request_was_sent() == false`), asserts sur le **booléen de
    classe**, jamais sur le texte — sinon le test réintroduirait exactement le
    couplage que D1 refuse.
14. Garde structurelle : un scan de source refusant le retour d'un
    `2>/dev/null` sur un appel `_arch_ask*` dans `_iterate_groom_loop`. Un test
    comportemental ne peut pas voir cette classe — la régression ne rendrait
    aucune décision fausse, elle rendrait le diagnostic aveugle pendant que
    toutes les assertions restent vertes (motif `mika#2131`).

### Étape 5 — documentation

15. `crates/mika-cli/CLAUDE.md` § `mika ask` : documenter `75` comme partie du
    contrat de sortie, avec la règle « la classe vient de la variante ».
16. `CLAUDE.md` racine : les deux variables d'environnement et les signaux
    grep, dans la forme des sections voisines.

---

## Verification contract

| # | Vérification | Attendu |
|---|---|---|
| V1 | `make test-dispatch-lib` | vert, y compris les trois nouveaux cas |
| V2 | `cargo test -p mika-cli` | vert ; `test_wrap_send_error_preserves_underlying_a2a_error_chain` toujours vert (mask-through mika#1985 intact) |
| V3 | `cargo clippy` / `cargo fmt` | propres |
| V4 | `mika ask` vers un port fermé | exit `75`, message inchangé |
| V5 | `mika ask` avec `--session-id` d'un autre agent | exit `1` (contrat, pas transport) |
| V6 | groom lancé, `mika-spirit` redémarré pendant la passe 1 | un `arch_ask_retry` puis convergence — pas de `PIPELINE_INCOMPLETE` |

---

## Definition of Done

- Les quatre sites d'appel passent par `_arch_ask_with_retry` et ne jettent plus
  stderr.
- `mika ask` distingue transport et contrat par le **code de sortie**, jamais
  par le texte.
- Le budget est de un retry, désarmable par variable d'environnement, avec un
  contrôle négatif par terme (code non-transport ; désarmement).
- Les deux nouveaux événements sont journalisés et documentés, avec leur régime
  attendu.
- `CLAUDE.md` racine et `crates/mika-cli/CLAUDE.md` à jour.
- `make test-dispatch-lib`, `cargo test -p mika-cli`, `cargo clippy`,
  `cargo fmt` verts.

---

## Acceptance criteria

*(Le ticket ne porte pas de section `## Acceptance criteria` ; dérivés des
Requirements et du Verification contract.)*

- **AC1** — Un `mika ask` dont l'échec est de classe transport sort en `75` ;
  un échec de contrat sort en `1`. Vérifiable par V4 et V5.
- **AC2** — La classification est décidée par la variante d'erreur et non par
  une sous-chaîne du message ; aucun `grep` sur le texte d'erreur n'apparaît
  dans `dispatch-lib.sh`. Vérifiable par lecture du diff et par les tests de
  l'étape 13.
- **AC3** — `_arch_ask` réessaie au plus une fois, uniquement sur `75`, après
  `MIKA_ARCH_ASK_RETRY_DELAY_SECS`. Vérifiable par les trois cas de l'étape 10
  à 12.
- **AC4** — Les quatre sites d'appel de `_iterate_groom_loop` capturent stderr
  et le WARN d'échec porte le message du CLI. Vérifiable par le diff et par la
  garde structurelle de l'étape 14.
- **AC5** — `MIKA_ARCH_ASK_RETRY=0` désarme le retry sans redéploiement du
  binaire. Vérifiable par le cas de l'étape 12.
- **AC6** — Aucun code de sortie existant ne régresse : les appelants
  `--task-complete` de `skills/bundled/**` sont inchangés en comportement.
  Vérifiable par V2 et par l'inventaire de D3.
- **AC7** — Un groom traversant un restart de `mika-spirit` converge au lieu de
  mourir en `PIPELINE_INCOMPLETE`. Vérifiable par V6.

---

## Surfaces opérateur

Dans `$MIKA_SPIRIT_LOG_FILE` (stderr du dispatch y aboutit) :

- `arch_ask_retry` (INFO) — **régime attendu : rare, non nul**. Chaque ligne est
  une passe d'architecte sauvée. Un flot soutenu ne se traite pas en allongeant
  le budget : il dit que le service redémarre trop souvent, ou qu'un agent
  refuse au lock, et c'est **cela** qu'il faut traiter.
- `arch_ask_retry_exhausted` (WARN) — **régime attendu : zéro**. Le retry a été
  dépensé et la passe est quand même perdue.

**Sonde post-déploiement, avec sa halte.** Sur 48 h, aucun
`first-pass _arch_ask failed` sans ligne `arch_ask_retry` correspondante. S'il
en reste : **halte, ne pas élargir la classe retryable** — cela signifie que
l'échec sort en `1`, donc qu'il n'est pas de classe transport, et c'est sa
classification qu'il faut établir d'abord. Symétriquement, si un
`arch_ask_retry` apparaît sur une erreur de contrat (session d'un autre agent,
message vide), la ligne de partage de D2 fuit : désarmer par
`MIKA_ARCH_ASK_RETRY=0` et réparer la classification, pas le budget.

---

## Hors périmètre, délibérément

- **Le flux opérateur `/mika-groom-ticket`.** Il vit dans `mika-platform`
  (M2) et son remède relève de l'enforcement par prompt. Ce plan réduit sa
  surface d'exposition sans le corriger : **ticket de suivi côté
  mika-platform**. À noter que la prévention opérationnelle que le ticket
  propose — séquencer les restarts hors grooms en vol — reste la bonne réponse
  pour ce flux-là.
- **Le message `Recovery::StillRunning → « Retry. »`.** Il est vrai à l'instant
  où il est écrit (M1). Le rendre prédictif demanderait au CLI de deviner un
  restart futur.
- **Le budget LLM et le plafond par appel** (mika#2189/#2293/#2297) : déjà
  traités, et ce plan n'en déplace aucune valeur.
- **Un retry sur les autres appelants de `mika ask`.** Les handlers
  `--task-complete` ont leur propre chemin de livraison et leur propre
  quarantaine (mika#2179) ; leur ajouter un budget ici superposerait deux
  mécanismes de réessai sur une même livraison.
- **La cause des restarts eux-mêmes.** Ce travail rend le groom survivant à un
  restart ; il ne réduit pas leur fréquence.

---

## Risques

| Risque | Portée | Mitigation |
|---|---|---|
| Un tour payé deux fois quand le premier aboutit finalement | quelques minutes, quelques centimes | budget de 1, et l'asymétrie de D2 rend ce coût très inférieur à celui d'une passe perdue |
| L'architecte voit le même prompt deux fois sur une session réutilisée | cosmétique | nommé en D5 ; c'est pourquoi le budget est de 1 |
| `75` lu comme un échec générique par un consommateur non recensé | contrat | inventaire en D3 : tous les autres appelants sont `--task-complete`, un chemin qui ne peut pas produire `75` |
| Le délai immobilise le slot de groom | throughput | 30 s contre une passe d'architecte de plusieurs minutes ; borné haut et bas, désarmable |
| La ligne de partage transport/contrat fuit | correction | contrôle négatif dédié (étape 11) + sonde post-déploiement avec halte |
