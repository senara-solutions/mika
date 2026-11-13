# fix(mika-qa) — un tour de revue dont le re-prompt a échoué aussi émet `qa_turn_ended_without_review` (AC2 de #2727, vol 1)

- **Ticket :** senara-solutions/mika#2732
- **Parent :** senara-solutions/mika#2727 (AC1 + AC3 livrés ; D6 de son plan porte le report d'AC2)
- **Type :** fix
- **Vol :** 1 sur 2 — la **tête** part en vol 2, pour la raison mesurée au § *Taille estimée*

---

## Contexte

#2727 a livré la garde : un tour `qa-review` ouvert par un webhook de PR qui
atteint `EndTurn` sans revue soumise est re-prompté **une fois**
(`agent_loop/mod.rs:3383` et `:4397`, consigne
`qa_review_submission::REVIEW_SUBMISSION_CORRECTION`). Le signal lu est celui que
la session pose déjà — `qa_build_callback::pr_review_posted_in_turn`, au niveau
du tour — et non le nom de l'outil, ce qui est tout l'objet du défaut : un
`run_gh` en lecture (`pr list`, `pr view`) satisfaisait le terme `run_gh` de
`required_tools` alors que rien n'atteignait GitHub. Deux tours de #2704 sont
partis ainsi le 2026-10-15 et ont exigé une relance humaine.

Ce qui reste ouvert est le cas où **le re-prompt échoue aussi**. Il est déjà
épinglé comme comportement — `ac1_le_re_prompt_est_a_un_coup_la_garde_ne_boucle_pas`
dans `tests/eval/test_qa_review_submission_2727.rs` prouve que le tour **sort**
plutôt que de boucler — mais il sort en **silence**. Aucune ligne ne dit à
l'opérateur qu'une PR vient de traverser deux tentatives de revue sans en
recevoir une, et c'est exactement la forme que mika#2272 nomme : *une population
qu'on ne compte pas se lit comme une flotte en bonne santé*.

Le complémentaire qui arme cet événement **existe déjà et est testé** :
`qa_review_submission::submission_unmet_after_retry(due, retries, summaries)` —
même conjonction que la garde, le terme de budget inversé
(`qa_review_submission.rs:158`, test
`mika2727_le_complementaire_sarme_quand_le_re_prompt_a_echoue_aussi`). Il n'a
**aucun appelant en production**. Ce vol lui en donne trois, un par chemin de
sortie EndTurn, et pose la ligne.

---

## Décisions de grooming

### D1 — Le découpage : l'**événement** d'abord, la **tête** ensuite

AC2 entier (lecteur étendu + appelants migrés + émetteur + tests) a été estimé à
≈ 290 lignes, au-dessus de la borne par vol de 243 (§ *Taille estimée*). Il se
coupe donc en deux, et **l'ordre choisi n'est pas celui que l'énoncé du ticket
suggère**. Deux raisons, toutes deux structurelles :

1. **Un vol « lecteur d'abord » livrerait un champ que personne ne lit.**
   Étendre `parse_pr_action_event` d'un quatrième terme sans son consommateur
   produit un `branch` jamais lu hors `#[cfg(test)]` : clippy le signale en
   `dead_code`, et le remède usuel — `#[allow(dead_code)]` — est précisément la
   forme qu'on ne veut pas voir atterrir. Un vol « événement d'abord » n'a pas
   ce problème : chaque ligne livrée a un lecteur le jour où elle atterrit.
2. **Le vol 1 est déjà actionnable sans la tête.** `repo` + `pr` identifient la
   PR ; `gh pr view <pr> --repo <repo> --json headRefName,headRefOid` rend la
   branche *et* le SHA en un appel que l'opérateur fait de toute façon pour
   regarder la PR. Ce que le vol 2 achète n'est donc pas l'actionnabilité d'une
   ligne, c'est la **lecture en agrégat** — greper les branches qui récidivent
   sur une flotte de tours, où la recherche manuelle par ligne ne tient pas.

**Ce vol sous-livre donc AC2.2 sur un point, et le dit** : l'événement ne porte
pas de champ de tête. Il n'en porte **aucun**, pas un champ vide — motif
mika#2331, *absent ≠ vide* : un `branch=""` ou un `branch="unknown"` posé par ce
vol se relirait comme « la branche a été cherchée et n'existe pas », ce qui est
faux. La suite est nommée par la ligne `Tracked in:` du § *Hors périmètre*.

### D2 — La tête est la **branche**, et le champ s'appelle `branch` (AC2.3)

AC2.3 demande de dire ce que « tête » veut dire ici. La réponse est mesurée, pas
choisie : **le SHA de tête n'est sur aucun fil.** La gateway écrit
`[GitHub] PR {action}: {repo}#{number} — {title} (branch: {branch})\n{url}`
(`mika-gateway/src/github.rs:505`), le caveat est déjà documenté à
`server/webhook_queue_v2.rs:62`, et ni la clé `qa_pr_sync_observed`
(`pr:{repo}#{pr}`) ni la clé de dédup de session #821 n'en portent. La seule
identité de tête disponible sur le fil est la **branche**.

Le champ s'appellera donc `branch` — jamais `head`, ni `head_branch`. AC2.3 le
demande littéralement, et la raison tient : un champ nommé `head` qui contient
une branche se relit comme un SHA manquant. Ceci **diverge du nom
`head_branch: Option<&str>` que D6 du plan de #2727 avait pré-écrit**, et c'est
délibéré : ce nom ré-importe le mot que l'AC demande d'écarter d'une valeur qui
n'est pas un SHA.

### D3 — Pas de requête `gh` pour obtenir le SHA, et la raison est la correction

AC2.3 pose explicitement le coût d'un SHA — une requête `gh` de plus par
événement — et demande de trancher dans le grooming. **Tranché : non**, et
l'argument décisif n'est pas le coût.

Une requête à l'instant de l'émission rend le head **maintenant**, pas le head
que le tour a revu. Or l'incident fondateur de #2727 est *littéralement* un
déplacement de tête en cours de route : deux tours sur deux têtes différentes
(`4a216bf7` à 15:11:52Z, `3f2c3fa4` à 19:11:36Z). Une poussée pendant le tour
ferait donc nommer à l'événement un SHA que le tour n'a jamais vu, avec l'allure
d'une mesure. Un instrument qui a l'air de mesurer et qui devine est pire que
pas de champ (motif mika#2293).

S'y ajoutent deux coûts ordinaires : un appel réseau sur un chemin de sortie du
loop, et un mode de panne de plus sur un diagnostic. Si un SHA devient
nécessaire, sa source correcte est ce que **le tour lui-même** a observé — la
sortie de son `qa_pr_view`, ou le stamp `QA_REVIEW_PR_TARGET_KEY` de mika#2368 —
jamais une requête fraîche.

### D4 — Un seul émetteur, pas trois `warn!`

`submission_unmet_after_retry` est le site d'armement exigé par AC2.2 ; il ne
suffit pas, parce que trois sites de sortie veulent la même ligne et que trois
copies d'un jeu de champs divergent aussi sûrement que trois copies d'une
conjonction (leçon mika#2158). Le vol livre donc **une** fonction,
`qa_review_submission::emit_turn_ended_without_review(...)`, qui appelle
`submission_unmet_after_retry` en interne et rend `()` quand la population n'est
pas la sienne. Chaque site de sortie est un appel de trois lignes, aucun ne
porte de prédicat ni de nom de champ.

La conjonction reste donc à un site, le jeu de champs est à un site, et aucune
quatrième copie n'apparaît — ce que AC2.2 demande mot pour mot.

### D5 — Les **trois** sorties EndTurn, et pas les sorties coupées

L'émetteur est posé aux trois chemins où le filet frère de mika#2368 pose déjà
`qa_build_callback::verdict_unmet_after_retry` — `agent_loop/mod.rs:4289`
(texte non vide), `:4533` (texte vide) et `:4756` (tour à outils seuls, EndTurn
forcé). Par construction ce sont **exactement** les sorties EndTurn de
`run_loop`, et un tour n'en emprunte qu'une : chacune est un `return`.

Les trois, et non celle du défaut mesuré seule, pour une raison de lecture : le
site `:4533` est sous `!mode.follow_up_on_empty()`, donc silencieux-seulement,
et notre population est conversationnelle (`server::handlers::run_agent_for_message`
→ `run_agent`). Câbler les trois évite qu'un futur éditeur ait à re-dériver
quelle enveloppe atteint quelle sortie ; le terme `due` de la conjonction garde
les sites hors population muets sans qu'on ait à le promettre.

**Les sorties coupées sont hors périmètre, et c'est une décision, pas un oubli.**
La deadline et le plafond de pas sortent par
`verdict_unmet_at_cut_off` — prédicat **sans** terme de budget, parce qu'un tour
coupé n'a jamais consommé le re-prompt. AC2.2 nomme sa population : *« un tour de
revue dont le re-prompt a échoué »*. Un tour coupé n'en fait pas partie, et il
est déjà couvert ailleurs : `server::deadline_verdict` (mika#2276, mika#2289,
mika#2515) pose un verdict sur une PR dont le tour a été coupé.

### D6 — WARN, `target: "mika::otel"`, un champ `event`

La convention du dépôt pour une ligne destinée à l'opérateur :
`warn!(target: "mika::otel", event = "qa_turn_ended_without_review", …)`. C'est
la forme que portent déjà `qa_switch_invalid`, `qa_review_reconcile_*`,
`qa_ci_coherence_refused` et les autres. Aucun recensement de noms d'événements
n'existe dans ce crate — le nom est donc fixé par une constante publique,
`TURN_ENDED_WITHOUT_REVIEW_EVENT`, pour que la sonde du § *Mesure
post-déploiement* et le test eval citent la même chaîne que l'émetteur.

### D7 — La capture de log en test, et la moitié qui porte (mika#2646)

Le test eval installe un abonné de capture. Deux contraintes, dont la seconde
est celle qu'on oublie :

- `tracing::subscriber::set_default` depuis `tests/` est hors du scan U3
  (`mika2646_set_default_a_un_site_dinstallation_unique` n'énumère que `src/`),
  donc l'helper de capture est local au fichier, comme
  `tests/eval/test_context_scope_observability_2305.rs` le fait déjà. Il
  emprunte aussi son garde-dispatcher (`keep_registry_multi_dispatcher`) : sans
  lui, un callsite atteint en premier par un thread sans abonné met en cache
  `Interest::never` **globalement**, et la capture rend zéro.
- `#[serial_test::serial]` va sur le test capturant **et sur tout voisin qui
  atteint le même callsite**. Ici le voisin existe et il est dans le même
  fichier : `ac1_le_re_prompt_est_a_un_coup_la_garde_ne_boucle_pas` rejoue
  exactement la séquence qui fait tirer l'émetteur. C'est la moitié porteuse
  (doc-comment de `install_capturing_subscriber`), et l'oublier produit un test
  vert par intermittence.

---

## Conception

### `crates/mika-agent/src/qa_review_submission.rs` — l'émetteur

```rust
/// Nom de l'événement opérateur (AC2.2). Constante parce que la sonde du
/// runbook, le test eval et l'émetteur doivent citer la même chaîne.
pub const TURN_ENDED_WITHOUT_REVIEW_EVENT: &str = "qa_turn_ended_without_review";

/// Un tour de revue a traversé la garde **et** son re-prompt sans rien
/// soumettre : le dire, une fois, à l'opérateur.
///
/// Arme sur [`submission_unmet_after_retry`] — le complémentaire exact de la
/// garde, et le seul site de cette conjonction (AC2.2). Hors population, rend
/// `()` sans rien écrire.
///
/// Les champs : `repo` et `pr` lus par le **lecteur unique** de la grammaire de
/// fil, `server::webhook_queue_v2::parse_pr_action_event`. Aucun champ de tête
/// dans ce vol — pas vide, **absent** (D1, motif mika#2331).
pub fn emit_turn_ended_without_review(
    due: bool,
    retries: &HashSet<&'static str>,
    summaries: &[ToolCallSummary],
    user_input_text: &str,
    trace_id: &str,
    agent_id: &str,
) {
    if !submission_unmet_after_retry(due, retries, summaries) {
        return;
    }
    let Some((_action, repo, pr)) = crate::server::webhook_queue_v2::parse_pr_action_event(
        user_input_text,
    ) else {
        // Injoignable en pratique — `due` exige déjà que le message parse —
        // mais le dire est moins cher que de poser une ligne sans PR.
        return;
    };
    warn!(
        target: "mika::otel",
        event = TURN_ENDED_WITHOUT_REVIEW_EVENT,
        repo = %repo,
        pr = pr,
        trace_id = %trace_id,
        agent_id = %agent_id,
        "qa-review turn ended without a submitted review, after the re-prompt failed too"
    );
}
```

Pas de second extracteur : l'appel passe par le lecteur unique, et le scan
structurel qui l'exige existe déjà — `mika2649_aucun_second_parseur_de_grammaire_devenement`
(`canonical_tokens.rs`) confronte la population des `Regex::new(` portant
`[GitHub]` à son recensement, **dans les deux sens**. Ce vol n'y touche pas et
n'a rien à y ajouter : il n'écrit aucune regex.

### `crates/mika-agent/src/agent_loop/mod.rs` — les trois sites

Chaque site, juste avant son `return Ok(LoopResult::Done { … })`, à côté du bloc
`verdict_unmet_after_retry` frère :

```rust
// mika#2732 — le re-prompt de la garde mika#2727 a échoué aussi : le dire.
crate::qa_review_submission::emit_turn_ended_without_review(
    qa_review_due,
    &intent_guard_retries,
    &all_tool_summaries,
    &user_input_text,
    tool_ctx.trace_id,
    db.agent_id(),
);
```

`qa_review_due` est déjà calculé une fois en tête de `run_loop`
(`agent_loop/mod.rs:1579`), à partir du même `user_input_text` que la garde lit —
donc la garde et l'événement ne peuvent pas diverger sur ce qu'est un webhook de
PR.

### Chemin de test (AC2.4 a + b)

Les deux moitiés dans **un seul `#[test]`**, comme AC2.4(b) le demande
(« contrôle négatif dans le même test »), dans
`tests/eval/test_qa_review_submission_2727.rs` — le fichier qui porte déjà les
fixtures (`pr_synchronize_event`, `RecordingRunGh`, `StubQaPrViewTool`,
`qa_review_skill`), donc zéro ligne de fixture neuve :

1. installer la capture (une fois, pour les deux tours) ;
2. **tour A** — la séquence de `ac1_…` : `qa_pr_view` → `run_gh pr list` → texte
   → [correction] → texte. Rien de posté ;
3. **tour B** — la séquence de `ac3b_…` : `qa_pr_view` → `run_gh pr review` →
   texte. Revue postée ;
4. assertions : **exactement une** ligne portant
   `event = "qa_turn_ended_without_review"`, et elle porte
   `repo = "senara-solutions/mika"` et `pr = 2704`.

La cardinalité est l'assertion qui compte : elle dit d'un coup que le tour A
émet et que le tour B n'émet pas. Deux tests séparés ne diraient pas que le
contrôle négatif a tourné sous la même capture.

---

## Étapes d'implémentation

1. `qa_review_submission.rs` : `TURN_ENDED_WITHOUT_REVIEW_EVENT` +
   `emit_turn_ended_without_review`, avec le doc-comment qui porte D1 (pas de
   champ de tête, et pourquoi), D3 (pas de SHA, et pourquoi) et D5 (pourquoi
   trois sites et pas les coupures).
2. `qa_review_submission.rs` : tests unitaires de population — hors population
   rien n'est écrit, budget intact rien n'est écrit, revue postée rien n'est
   écrit. Assertions sur le prédicat d'armement (le `warn!` est couvert par le
   test eval, pas re-testé ici).
3. `agent_loop/mod.rs` : les trois appels (`:4289`, `:4533`, `:4756`), chacun
   accolé au bloc `verdict_unmet_after_retry` existant.
4. `tests/eval/test_qa_review_submission_2727.rs` : helper de capture local
   (garde-dispatcher inclus) + le test à deux tours, `#[serial_test::serial]` sur
   lui **et sur `ac1_le_re_prompt_est_a_un_coup_la_garde_ne_boucle_pas`** (D7).
5. Documenter la sonde : une entrée dans le § *Mesure post-déploiement* reprise
   dans le corps de PR. Pas de nouvelle variable d'environnement, donc rien à
   écrire dans `docs/operator/environment-variables.md`.
6. Suite complète, clippy, fmt, lints du dépôt, `scripts/verify-pipeline.sh origin/main`.
7. Corps de PR : **ne pas** écrire `Closes #2732` — le ticket reste ouvert pour
   le vol 2 (précédent mika#2699). Écrire la ligne `Tracked in:` du § *Hors
   périmètre*.

---

## Contrat de vérification

- **W1** — `cargo test -p mika-agent qa_review_submission` : les prédicats de
  #2727 restent verts **sans modification d'assertion** (ce vol n'en change
  aucun), et les tests de population de l'émetteur passent.
- **W2** — `cargo test -p mika-agent --test eval qa_review_submission_2727` :
  le nouveau test rend une ligne et une seule ; `ac3a`, `ac3b` et `ac1`
  inchangés.
- **W3** — `cargo test -p mika-agent --test eval qa_build_callback_verdict_2355`
  vert sans modification : le filet frère de mika#2368 partage les trois sites
  de sortie et n'a pas changé de comportement.
- **W4** — `cargo test -p mika-agent canonical_tokens` vert :
  `mika2649_aucun_second_parseur_de_grammaire_devenement` et
  `mika2646_set_default_a_un_site_dinstallation_unique` n'ont pas bougé — ce vol
  n'écrit ni regex ni `set_default` sous `src/`.
- **W5** — `cargo test -p mika-agent --test eval` **deux fois de suite** verts :
  la capture de log est la source de flakiness connue (mika#2646), et une
  exécution unique ne la mesure pas.
- **W6** — suite complète, `cargo clippy`, `cargo fmt --check`, lints du dépôt,
  `scripts/verify-pipeline.sh origin/main` verts.

---

## Fire-Disposition

Ce vol livre deux détecteurs : l'émetteur lui-même (chemin de succès = aucune
ligne, c'est-à-dire « aucune violation trouvée ») et le test eval. Option
retenue : **(a) exception nommée en allowlist**, dans sa forme à population
vide — la forme idiomatique du dépôt (mika#2425 V1, mika#2025 V10, et le plan de
#2727 pour le même module).

- **Aucune allowlist n'est créée, et c'est vérifiable plutôt que promis.**
  L'émetteur est un détecteur *de temps d'exécution* : sa population est
  l'ensemble des tours à venir, pas un corpus de fichiers existants. Il n'y a
  donc aucune violation à exempter au moment du merge — pas « on a choisi de ne
  pas les lister », mais « l'ensemble est vide par construction ».
- **La seule exception est celle, héritée et déjà nommée, de #2727** : le second
  terme de `review_submission_satisfied`, soit
  `build_dispatched_in_turn` (réserve D3 de #2727) — un tour qui lance
  `build_mika` finit légitimement sans revue, et mika#2355 prend le relais sur
  son callback. Elle est **par valeur nommée** (`BUILD_MIKA_TOOL`), grep-visible
  depuis le nom de la garde, et son assertion auto-nettoyante est la suite
  mika#2355 elle-même : si ce relais cesse d'exister,
  `test_qa_build_callback_verdict_2355` rougit avant que la réserve ne devienne
  un trou. Ce vol n'en ajoute aucune autre et n'en élargit aucune.
- **Tout atterrit armé.** Aucun `#[ignore]`, aucun `#[cfg(skip)]`, aucun
  désarmement conditionnel. Le `#[serial_test::serial]` de D7 est une contrainte
  d'ordonnancement, pas un désarmement : le test s'exécute à chaque run.

---

## Risques nommés

| # | Risque | Parade |
|---|---|---|
| R1 | **Le test capturant est vert par intermittence** (mika#2646) : un voisin atteint le callsite sans abonné et le cache `Interest::never` globalement. | D7 : garde-dispatcher + `#[serial]` sur le capturant **et** sur `ac1_…`, le voisin qui rejoue la même séquence. W5 exige deux exécutions consécutives vertes. |
| R2 | **Un site de sortie est oublié** et l'événement est muet sur une partie de sa population — un filet à trous se lit comme un filet qui n'a rien à faire. | D5 : les trois sites sont ceux où `verdict_unmet_after_retry` est **déjà** posé, donc l'énumération n'est pas refaite de zéro ; W3 vérifie que le filet frère est intact aux mêmes trois endroits. |
| R3 | **L'émetteur ajoute une ligne sur un tour sain.** | La conjonction est celle de la garde, terme de budget inversé, déjà testée (#2727). Le test eval pose le contrôle négatif sous la même capture : un tour qui poste sa revue n'émet rien. |
| R4 | **Le vol 2 se perd** et l'événement reste sans tête pour toujours. | `Tracked in:` sur mika#2732 lui-même, et le corps de PR **n'écrit pas** `Closes #2732` (étape 7). Le ticket reste ouvert, porteur du vol 2. |
| R5 | **Un futur éditeur ajoute un champ `head` vide** pour « compléter » l'événement avant le vol 2. | Le doc-comment de l'émetteur porte D1 et D2 : absent plutôt que vide, et `branch` jamais `head`. |

---

## Hors périmètre

- **La tête sur l'événement (vol 2).** Étendre `PR_ACTION_RE` d'un quatrième
  groupe optionnel, faire rendre à `parse_pr_action_event` une struct nommée
  `PrActionEvent<'a> { action, repo, pr, branch: Option<&'a str> }`, migrer ses
  trois appelants (`classify_event`, `server::deadline_verdict::parse_pr_target`,
  `qa_review_submission::is_review_triggering_event`), poser le champ `branch`
  sur l'événement, et le contrôle de non-vacuité d'AC2.4(c) — assertion sur la
  **valeur** de la branche extraite, pas sur sa présence. Estimé ≈ 105 lignes.
  Deux points déjà tranchés pour ce vol-là, pour qu'il n'ait pas à les
  re-découvrir : le groupe est **optionnel** (une ligne sans `(branch: …)` doit
  continuer à parser, et le champ vaut alors `None`, pas `""`) ; et il est
  **greedy** sur le préfixe, de sorte qu'un titre de PR contenant lui-même
  `(branch: …)` ne gagne pas contre le segment que la gateway appose en dernier.

  `Tracked in: senara-solutions/mika#2732` (le ticket reste ouvert ; ce vol ne
  le ferme pas).

- **Le `unknown` de la gateway.** `format_event_text` substitue le littéral
  `unknown` quand `pull_request.head.ref` est absent
  (`mika-gateway/src/github.rs:496`) : le fil a donc **déjà** aplati l'absence en
  valeur, et aucun lecteur en aval ne peut distinguer « pas de head ref » d'une
  branche réellement nommée `unknown`. La réparation est au producteur — omettre
  le segment plutôt que poser un placeholder — et elle touche une grammaire de
  fil que trois lecteurs partagent. Hors de ce vol et hors du vol 2 ; à ouvrir
  comme ticket propre si quelqu'un voit un `branch=unknown` en production.

- **Les sorties coupées** (deadline, plafond de pas). D5 : population distincte,
  déjà couverte par `server::deadline_verdict`.

- **La factorisation de l'helper de capture de log.** Sept copies de
  `CapturingLayer` vivent sous `crates/mika-agent/tests/` ; ce vol en ajoute une
  huitième plutôt que d'en migrer sept, parce que la migration est un périmètre
  à elle seule et qu'elle toucherait sept fichiers sans rapport avec ce ticket.
  Observation posée ici pour qu'elle ne se re-découvre pas une neuvième fois.

---

## Mesure post-déploiement

L'instrument n'est pas vivant avant `make deploy` (mika#2340) : l'absence de la
ligne sur un binaire antérieur signifie « binaire ancien », jamais « aucun tour
n'a échoué ».

**Sonde :**

```sh
grep -h 'qa_turn_ended_without_review' ~/.mika/logs/mika-spirit.log* \
  | jq -r '[.timestamp, .repo, (.pr|tostring), .trace_id] | @tsv'
```

**Table de lecture :**

| ce qu'on voit | ce que ça dit |
|---|---|
| aucune ligne, et des tours de revue ont tourné | le re-prompt de #2727 suffit : la garde se corrige au premier coup |
| une ligne de temps en temps | la population existe et est maintenant comptée — c'est le livrable |
| la **même** `pr` plusieurs fois | une PR que mika-qa ne parvient pas à revoir ; escalade opérateur, pas un défaut du filet |
| aucune ligne et le binaire est antérieur au déploiement | l'instrument n'existe pas encore sur cet hôte (mika#2340) |

**Contrôle positif :** le test eval de ce vol est le seul contrôle positif
disponible — un événement dont le chemin de succès est le silence ne peut pas
être distingué d'un émetteur mort par la seule lecture des logs. C'est la raison
pour laquelle la cardinalité est assertée en test plutôt que décrite ici.

---

## Definition of Done

- `emit_turn_ended_without_review` existe, s'arme sur
  `submission_unmet_after_retry`, et est appelé aux trois sorties EndTurn de
  `run_loop`.
- L'événement porte `event`, `repo`, `pr`, `trace_id`, `agent_id` ; il ne porte
  **aucun** champ de tête, ni vide ni placeholder.
- Aucun second extracteur de `[GitHub] PR …` n'apparaît ; aucune regex n'est
  écrite par ce vol.
- Le test eval à deux tours passe, avec `#[serial]` sur lui et sur `ac1_…`.
- W1–W6 verts, deux exécutions consécutives pour W5.
- Le corps de PR porte `Tracked in: senara-solutions/mika#2732` et **pas**
  `Closes #2732`.

---

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#2732, avec le partage par vol
explicité (D1).

- **AC2.1** — `parse_pr_action_event` rend la branche en plus de l'action, du
  dépôt et du numéro, en un seul lecteur. Ses appelants sont migrés, aucun second
  extracteur de `[GitHub] PR …` n'apparaît.
  → **Vol 2** pour la moitié « rend la branche » et la migration des appelants.
  **Ce vol** tient la moitié « aucun second extracteur » : il consomme le
  lecteur unique et n'écrit aucune regex (W4).
- **AC2.2** — un tour de revue dont le re-prompt a échoué aussi émet
  `qa_turn_ended_without_review` (WARN, visible opérateur), portant `repo`, `pr`
  et la tête disponible. Le site d'armement est
  `qa_review_submission::submission_unmet_after_retry`, pas une quatrième copie
  de la conjonction.
  → **Ce vol**, moins le champ de tête, qui part en vol 2 (D1). `repo`, `pr`, le
  niveau WARN et le site d'armement unique sont livrés ici.
- **AC2.3** — dire ce que « tête » veut dire ici. Si le champ porte la branche
  et non le SHA, il s'appelle `branch` et pas `head`. Si le SHA est jugé
  nécessaire, le coût est une requête `gh` supplémentaire par événement, à
  décider explicitement dans le grooming.
  → **Ce vol**, et c'est un AC de grooming : D2 (c'est la branche, le champ
  s'appellera `branch`) et D3 (pas de SHA, et la raison est la correction avant
  le coût) le tranchent tous les deux explicitement.
- **AC2.4** — tests. (a) Le tour de l'AC3a de #2727, prolongé d'un re-prompt qui
  échoue, émet l'événement. (b) Contrôle négatif dans le même test : le tour de
  l'AC3b (revue postée) ne l'émet pas. (c) Un contrôle de non-vacuité sur la
  branche extraite — la même forme de fil, l'assertion sur la valeur et non sur
  sa présence.
  → **(a) et (b) dans ce vol**, en un seul `#[test]` à deux tours sous une
  capture unique. **(c) en vol 2**, puisqu'il porte sur la branche extraite.
- **Taille** — estimée tests compris : § *Taille estimée* ci-dessous.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/qa_review_submission.rs` (constante + émetteur + docs) | 55 |
| `crates/mika-agent/src/agent_loop/mod.rs` (trois sites de sortie EndTurn) | 24 |
| tests unitaires (`qa_review_submission.rs`, population de l'émetteur) | 20 |
| tests eval (`tests/eval/test_qa_review_submission_2727.rs` : capture + AC2.4 a/b) | 85 |

Total estimé : 184 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Pourquoi ça tient, et pourquoi AC2 entier ne tenait pas.** AC2 au complet a
été mesuré à ≈ 290 lignes : les 184 ci-dessus plus ≈ 105 pour la tête (regex
étendue, struct `PrActionEvent`, trois appelants migrés, tests d'extraction avec
le contrôle de non-vacuité). 290 passe la borne de 243 ; 184 la passe sous avec
une marge de 59 lignes, soit ≈ 24 %.

Le découpage est en **vols**, pas en phases : le vol 1 est mergeable seul, livre
un événement que l'opérateur peut lire et actionner dès l'atterrissage (D1,
raison 2), et ne laisse derrière lui aucune ligne sans lecteur (D1, raison 1).
La suite est nommée au § *Hors périmètre* par
`Tracked in: senara-solutions/mika#2732`.
