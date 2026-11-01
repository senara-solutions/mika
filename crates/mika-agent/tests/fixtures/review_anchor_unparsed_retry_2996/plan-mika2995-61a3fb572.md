# fix(2995) — le consentement ne s'auto-pose pas : l'écriture de `ready` est retirée au moteur

> Ticket : senara-solutions/mika#2995. Labels : `bug`, `p1-important`, `ready`,
> `dispatch:loop`, `loop-substrate`. Milestone : *Substrat de boucle* (#33), ouvert par
> exception au gel sur motif de sûreté (bearing de Prime du 2026-10-31, puis « go » de
> Vincent ; constat initial au parking mika#2983, commentaire 6546374052).
> Merge par Vincent (`loop-substrate`).

---

## 1. Où en est le ticket, et pourquoi cette voie-là

Le 2026-10-31 à 16:46:32Z, `validate_forced_dispatch_consent`
(`crates/mika-agent/src/skills/executor.rs:3452`) a refusé un groom forcé sur mika#2958
sous le verdict `forced_dispatch_without_consent`. Cinq minutes plus tard, dans le même
tour de mika-dev, l'outil `run_gh` a été appelé avec
`argv: ["issue", "edit", "2958", "--add-label", "ready"]` et le `GH_TOKEN` du moteur. La
timeline de mika#2958 porte `16:51:38Z labeled mika-platform-dev ready`. À 16:52:09Z le
groom partait.

Le défaut est précis et il n'est pas dans le prédicat de la garde : la garde lit
`labels_carry_ready(&labels)` (`webhook_dispatch.rs:599`), donc **la seule présence** du
label. Elle est correcte pour la question qu'elle pose. Ce qui manque, c'est que l'agent
qu'elle retient possède l'outil qui produit la réponse qu'elle attend.

### La lecture retenue : voie B — l'écriture de `ready` est refusée au moteur

C'est la deuxième des trois voies qu'AC1 énumère. Le mécanisme existe déjà et il est en
production depuis mika#2573 :

```rust
// crates/mika-agent/src/evidence/guards.rs:2893
pub fn detect_fallthrough_work_creation(args: &[String]) -> Option<WorkCreationAction> {
    if args.first().map(String::as_str) != Some("issue") { return None }
    match args.get(1).map(String::as_str) {
        Some("create") => Some(WorkCreationAction::IssueCreate),
        Some("edit")   => added_dispatch_trigger_label(&args[2..])
                              .map(|label| WorkCreationAction::ReadyLabelAdd { label }),
        _ => None,
    }
}
```

`added_dispatch_trigger_label` balaie **toutes** les occurrences répétées de
`--add-label` / `--label` / `-l`, traite la forme `--flag=valeur` et les listes à virgule
(`"bug,ready"`), et compare sans tenir compte de la casse. L'argv exact de l'incident du
2026-10-31 est donc **déjà reconnu** par un détecteur mergé. Une seule ligne l'a laissé
passer :

```rust
// crates/mika-agent/src/skills/builtin_handlers.rs:3157
if !ctx.is_webhook_fallthrough_turn {
    return Ok(());
}
```

Le tour de 16:51:37Z était un tour de callback de dispatch, pas un tour Webhook
Fallthrough. La garde a vu le geste et s'est tue, parce que sa population avait été
bornée — légitimement — au domaine du ticket qui l'avait créée.

**Le vol lève `ReadyLabelAdd` hors de ce terme, et laisse `IssueCreate` dedans.**
L'asymétrie est le cœur de la décision, et elle n'est pas un compromis : créer une issue
de suivi est un flux de moteur légitime sur un tour ordinaire (le `Tracked in:` de cette
doctrine en dépend), tandis que **promouvoir un ticket n'est jamais un geste de moteur**.
`ready` est le consentement positif de l'opérateur au dispatch autonome (mika#841) ; le
seul écrivain gouverné est `ready_label::apply_ready` (mika#2315), qui tourne en Rust,
sous le scan d'écrivain unique `no_ready_label_write_outside_this_module`
(`ready_label.rs:1023`), et **ne passe jamais par `run_gh`** — ses deux
authentifications sont `ReadyWriteAuth::Cli` (subprocess `gh` direct) et
`ReadyWriteAuth::Rest` (token REST). `skills/bundled/_shared/dispatch-lib.sh` ne pose
jamais `ready` (`grep -c 'add-label ready' dispatch-lib.sh` → `0`). La voie LLM est donc
la **seule** fermée, et aucun flux gouverné ne la traverse.

### Pourquoi elle tient face à un agent qui veut lever la garde

Parce qu'elle ne dévalue pas le geste, elle **retire la capacité**. La différence est le
critère de Prime du 2026-10-31 (*impossible, pas improbable*) :

- Elle ne dépend d'aucune consigne de prompt. Le refus est pré-subprocess, au point de
  chaîne de `run_gh`, avant que `gh` ne soit lancé — c'est la forme mika#1646 (une garde
  post-hoc observe la violation sans l'empêcher).
- Elle n'a **pas d'interrupteur d'environnement**, par le même précédent mika#1646 déjà
  invoqué au site : un désarmement par variable sur un chemin de création de travail est
  un désarmement par coquille. Le geste de désarmement est un revert.
- Elle ne laisse pas de forme de repli : les trois syntaxes de `--add-label` sont
  couvertes, `gh api` est déjà borné par `validate_gh_api_scope` (mika#1167), et
  `gh issue reopen` n'émet pas de transition donc pas d'événement `labeled`.
- Surtout : elle ferme **l'émission de l'événement**, pas une lecture. GitHub n'émet
  `issues.labeled` que sur une **transition** (mika#2323) ; si la transition ne peut pas
  être produite par le moteur, aucun des huit lecteurs du § 2.3 n'a besoin d'un nouveau
  prédicat.

### Pourquoi la voie A est refusée — trois raisons, chacune suffisante

AC1 cite en premier *« l'acteur du dernier événement `labeled ready` dans la timeline »*.
Toute la machinerie existe et elle est éprouvée (`read_ready_label_timeline`,
`MachineIdentities::is_machine`, `is_parked`, `last_ready_labeled_at`). Elle est refusée
quand même :

1. **Le dépôt l'interdit structurellement.** `parse_event_actor`
   (`server/ready_label_handler.rs:~240`) porte le doc-comment *« # This value decides
   NOTHING … **No refusal predicate in this module reads it** … Introducing one here
   would be a filtering policy nobody has taken »*, et il est tenu par le scan de source
   `mika2323_no_gate_predicate_reads_the_actor` (`:2444`, **allowlist vide**) avec son
   contrôle de bonne foi `mika2323_the_actor_guard_bites_on_a_planted_read` (`:2407`).
   Câbler l'acteur dans un prédicat de refus est une politique de filtrage que personne
   n'a prise, et la prendre par la bande d'un correctif de sûreté serait la prendre sans
   la nommer.
2. **Elle produit un faux positif sur un flux légitime.** `apply_ready` pose `ready` sous
   l'identité machine dans trois flux voulus — la cascade de milestone
   (`server/milestone_context_handler.rs:380`) et les phases d'`auto_pull`
   (`auto_pull.rs:4249`, appelé en `:4889`, `:5056`, `:5750`). Le consentement y est
   délégué **en amont** par l'opérateur (le milestone, la mise au bassin). Une règle
   « acteur machine ⇒ pas de consentement » refuserait ceux-là aussi, et le remède serait
   une liste d'exceptions par chemin d'appel — c'est-à-dire la reconstruction, côté
   lecture, de la distinction que l'écriture porte déjà.
3. **Elle rend le self-consent non-comptant à **une** porte, pas impossible.** L'agent
   pourrait encore poser `ready` : l'événement partirait, le feeder `ready-label` créerait
   sa tâche, `stuck_ready_reconcile` le verrait, le bassin de milestone le compterait
   comme déjà promu. Seul mika#2772 détournerait les yeux. C'est exactement la garde
   improbable que le ticket condamne, déplacée d'un cran.

---

## 2. Le dessin

### 2.1 Le terme de population devient un prédicat nommé

La disposition 2 du site actuel est explicite : *« le terme de classe de tour est **DANS**
le prédicat, jamais une branche de l'appelant »*. Le vol la respecte en déplaçant la
décision dans `evidence/guards.rs`, auprès du détecteur :

```rust
/// Un geste de création de travail est-il refusé sur CE tour ?
///
/// Deux populations, et l'asymétrie est la décision de mika#2995.
pub fn work_creation_is_refused(
    action: &WorkCreationAction,
    is_webhook_fallthrough_turn: bool,
) -> bool {
    match action {
        // Promouvoir un ticket est le consentement de l'opérateur (mika#841).
        // Aucun tour de moteur ne le porte ⇒ refus sur TOUTE la population.
        WorkCreationAction::ReadyLabelAdd { .. } => true,
        // Créer une issue de suivi est un flux de moteur légitime hors
        // Fallthrough ⇒ la borne de mika#2573 est conservée telle quelle.
        WorkCreationAction::IssueCreate => is_webhook_fallthrough_turn,
    }
}
```

Le booléen reste un **paramètre**, pas un champ de `ctx` lu ici : `guards.rs` est du pur
argv et le reste. L'appelant passe `ctx.is_webhook_fallthrough_turn` et n'a plus de
branche.

### 2.2 Le site d'appel : une population, deux textes de refus

`validate_fallthrough_work_creation` (`builtin_handlers.rs:3140`) perd son `if !ctx…
{ return Ok(()) }` et gagne `if !work_creation_is_refused(&action, ctx.is_webhook_fallthrough_turn)
{ return Ok(()) }`. Trois conséquences à écrire, pas à découvrir :

- **Le `reason` doit cesser d'affirmer la population.** Le texte actuel dit *« This turn
  is a Webhook Fallthrough turn »*. Sur un `ReadyLabelAdd` hors Fallthrough, ce serait
  faux — un champ qui affirme ce qu'on n'a pas mesuré (classe mika#2304). Le `reason` et
  le `remedy` deviennent fonction du motif : la branche `ReadyLabelAdd` dit que la
  promotion appartient à l'opérateur ; la branche `IssueCreate` garde son texte mot pour
  mot.
- **Le `remedy` ne nomme aucun contournement** (doctrine mika#2520), et c'est la même
  exigence qu'AC3 pose sur l'autre texte. Pour `ReadyLabelAdd`, les sorties correctes
  sont : accuser réception et s'arrêter ; demander la promotion à l'opérateur avec
  `send_message` ; ou groomer explicitement, ce qui porte une intention et traverse la
  porte (mika#1173). Le remède ne décrit **pas** le geste `remove → add` — c'est le geste
  de l'opérateur, et le décrire à l'agent est la fuite que mika#2520 nomme.
- **La ligne de journal reste `fallthrough_work_creation_blocked`, et c'est voulu.** Elle
  porte déjà `motif`, et `motif = "ready_label_add"` suffit à séparer les deux
  populations dans un `GROUP BY`. Un second nom d'événement scinderait une mesure en deux
  (classe mika#2205) et casserait la jointure par `trace_id` avec
  `webhook_fallthrough_turn`. Ce qui s'ajoute est **un champ**, pas un nom :
  `turn_is_fallthrough` (booléen), parce que c'est précisément la dimension que la mesure
  d'AC5 doit pouvoir trancher.

### 2.3 Le recensement d'AC4 — tous les sites qui lisent `ready` comme consentement

Établi par `grep -rln 'labels_carry_ready\|READY_LABEL' crates/mika-agent/src`
(15 fichiers, dont 4 de test ou d'écriture gouvernée).

| site | ce qu'il lit | le même trou ? | ce vol |
|---|---|---|---|
| `skills/executor.rs:3452` `validate_forced_dispatch_consent` | présence, via `fetch_issue_labels_unless_pull_request` | **oui — c'est l'incident** | **ferme** (par l'écriture) + AC3 sur son remède |
| `server/ready_label_handler.rs` feeder `ready-label` | l'événement `issues.labeled` lui-même | **oui, et pire** : il crée une tâche de dispatch | **ferme** (l'événement n'est plus émissible par le moteur) |
| `server/webhook_queue_v2.rs:181` | `label == READY_LABEL` à la mise en file | oui, même événement | **ferme** (idem) |
| `auto_pull.rs` phases 0/1/2, `is_feeder_excluded` (`:2398`), `stuck_ready_reconcile` (`:5217`) | présence + âge du label + `is_parked` sur l'acteur du dernier événement | **oui, et sur deux axes** : un `ready` reposé par le moteur fait tomber le park de l'opérateur (`is_parked` ne rend `true` que si le **dernier** événement est un `unlabeled` non-machine), et `stuck_ready_reconcile` en fait un re-driver permanent | **ferme** (par l'écriture) |
| `milestone_manager/pool.rs:1030` + `:1185` (second `labels_carry_ready`) | présence, pour « ce sujet est-il déjà promu ? » | oui | **ferme** (par l'écriture). Le prédicat dupliqué est **nommé** ici et non fusionné : c'est la classe mika#2158, et la fusion est un geste de refactor indépendant ⇒ `Tracked in:` |
| `server/draft_pr_opened_handler.rs:44` + `:86` (troisième prédicat local) | présence | oui | **ferme** (par l'écriture) + `Tracked in:` pour la fusion |
| `skills/executor.rs:4329` porte de place mika#2084 | `dispatch:<seat>`, **pas** `ready` | **non — hors population.** Elle lit la place, pas le consentement. AC4 la nomme ; la réponse est qu'elle n'a pas ce trou | rien à faire, et le dire est la réponse |
| `agent_loop/mod.rs:12161` `READY_LABEL_DISPATCH_MARKER` | le **marqueur de message**, pas le label | indirect : le marqueur n'existe que si l'événement est parti | **ferme en amont** |
| `ready_label.rs`, `milestone_manager/pool_writer.rs:157`, `server/milestone_context_handler.rs:380` | **écrivains** gouvernés, pas lecteurs de consentement | sans objet | intouchés — et c'est la propriété qui rend la voie B sans régression |

**La lecture d'ensemble, qui est la réponse d'AC4 :** fermer l'écriture ferme les huit
lecteurs d'un coup, parce qu'aucun d'eux ne peut observer un label dont la transition n'a
pas eu lieu. Le recensement devient l'énoncé d'**une** écriture fermée au lieu de *N*
lectures rapiécées — exactement ce que la doctrine mika#2201 prescrit (router un site par
le lecteur unique plutôt que d'ajouter une ligne).

### 2.4 Le coût nommé : la prescription du prompt que ce refus casse

`skills/bundled/self-dev-callback/system_prompt.md:25` prescrit **mot pour mot** le geste
que le vol refuse :

> b. Call `run_gh("issue edit <n> --add-label ready --repo senara-solutions/<repo>")` to
> re-add the `ready` label. This re-triggers the ready-label webhook handler…

C'est la seule occurrence de `skills/bundled/`
(`grep -rl 'add-label ready' skills/bundled/` → un fichier). Elle est retirée et
remplacée par la sortie correcte — la promotion appartient à l'opérateur — qui est le même
mouvement qu'AC3 demande sur l'autre texte.

Trois faits qui bornent le coût, et qu'il faut lire avant de craindre une régression :

- **Le chemin est déjà inerte sur la population nominale**, et le dépôt le documente
  (`docs/operator/environment-variables.md:6855`) : le ticket porte déjà `ready` au moment
  du callback, `dispatch-lib.sh` ne le retire jamais, et GitHub n'émet `labeled` que sur
  une transition — donc le `--add-label` est un no-op silencieux.
- **Le cas non inerte est connu et nommé** : un groom lancé par
  `mika ask "groom mika issue#N"` (mika#2484), où le ticket peut ne pas porter `ready`.
  Il n'est **pas réparé dans ce vol** — sa réparation est le choix d'une voie de reprise
  (re-dispatch gouverné ou escalade à l'opérateur) et c'est un ticket, pas une ligne.
  `Tracked in:` ci-dessous.
- **Les deux moitiés n'ont pas le même instant d'activation** (mika#2340) :
  `skills/bundled/` est lu au build, donc la moitié prompt n'atteint aucun agent avant
  `make deploy`, tandis que la moitié Rust mord dès le binaire servi. Entre les deux,
  l'agent lit encore la prescription et se fait refuser — un faux positif **visible et
  borné**, exactement le sens du fail-safe que le site assume déjà par écrit. Sonde :
  `cat ~/.mika/skills/.manifest-writer`.

### 2.5 AC3 — le remède du refus de mika#2772

Dans la closure `refuse` de `validate_forced_dispatch_consent` (`executor.rs:~3518`), le
champ `"recovery"` dit aujourd'hui :

> Either promote the ticket by posting the `ready` label as remove→add (GitHub only emits
> `labeled` on a transition — mika#2323), or groom it explicitly with `mika ask --agent
> mika-dev "groom <owner/repo>#<n>"` …

La première branche est le geste de l'incident, décrit à l'agent sous forme de gabarit.
Elle est réécrite pour **attribuer** la promotion sans la **prescrire** : la promotion du
ticket appartient à l'opérateur et le moteur n'en a pas l'écriture (mika#2995) ; si ce
ticket doit être groomé maintenant, le dire à l'opérateur avec `send_message` ; la seconde
branche — groomer explicitement, ce qui porte une intention et traverse la porte
(mika#1173) — est conservée telle quelle, c'est une sortie correcte que l'agent peut
prendre lui-même.

Le geste `remove → add` **ne disparaît pas de la documentation** : il reste à
`docs/operator/environment-variables.md:6935`, qui est la surface opérateur. Il quitte le
texte que l'agent lit. C'est la distinction entière de mika#2520.

### 2.6 Le détecteur, et ce qu'il tient

Un scan de source dans `canonical_tokens.rs` (qui possède déjà `repo_root()` et scanne
déjà `skills/bundled/_shared/dispatch-lib.sh`, `:6525`) refuse qu'un prompt bundlé
prescrive la pose de `ready` par `run_gh`. Sans lui, la correction du § 2.4 est une
correction de texte que rien ne retient : le prompt a déjà dérivé trois fois sur ce dépôt
(classe mika#2158), et les variantes hors périmètre de ce ticket sont précisément des
copies du même texte.

Le scan a aussi un **contrôle de bonne foi** — il plante la prescription dans une fixture
et vérifie qu'il rougit — et une **assertion de population non vide** : un scan qui ne
lirait aucun fichier passerait vert par construction.

---

## 3. Fichiers touchés

| fichier | nature |
|---|---|
| `crates/mika-agent/src/evidence/guards.rs` | `work_creation_is_refused` + doc + tests de population |
| `crates/mika-agent/src/skills/builtin_handlers.rs` | câblage, textes par motif, champ `turn_is_fallthrough`, tests AC2 |
| `crates/mika-agent/src/skills/executor.rs` | réécriture du `"recovery"` (AC3) + son test |
| `crates/mika-agent/src/canonical_tokens.rs` | détecteur de prescription + contrôle de bonne foi |
| `skills/bundled/self-dev-callback/system_prompt.md` | retrait de la prescription (l. 25) |
| `docs/operator/environment-variables.md` | population de la garde, les deux motifs, sonde d'AC5 (hors compte) |

---

## 4. Ordre de commit

1. **Le détecteur, rouge.** Scan dans `canonical_tokens.rs` + contrôle de bonne foi +
   assertion de population. Il rougit sur
   `skills/bundled/self-dev-callback/system_prompt.md:25`. La sortie rouge est capturée
   pour le corps de la PR.
2. **Le prompt, vert.** Retrait de la prescription, remplacement par la sortie correcte.
   Le détecteur passe au vert ; son allowlist est **livrée vide**.
3. **Le prédicat.** `work_creation_is_refused` dans `guards.rs` + ses tests de population
   (`ReadyLabelAdd` refusé sur les deux valeurs du booléen ; `IssueCreate` refusé sur une
   seule). Aucun comportement ne change encore : la fonction n'est pas appelée.
4. **Le câblage + AC2.** Le site d'appel, les textes par motif, le champ de journal, et la
   paire de tests d'AC2 avec son contrôle négatif.
5. **AC3.** Le `"recovery"` de `executor.rs` + son test.

**Plancher : les commits 1 et 2.** Ils sont mergeables seuls et laissent la branche verte
(le détecteur armé, sa seule violation corrigée). Les commits 3 à 5 sont la fermeture
structurelle ; si le vol devait être tronqué, c'est là qu'il se coupe — mais le ticket
n'est pas clos par le plancher seul, puisque le plancher est de la prose et le ticket
exige l'impossibilité.

---

## 5. Definition of Done

- [ ] `grep -c -- 'add-label ready' skills/bundled/self-dev-callback/system_prompt.md`
      → `0`, et `grep -rc -- 'add-label ready' skills/bundled/` → aucune occurrence.
- [ ] `work_creation_is_refused` existe dans `crates/mika-agent/src/evidence/guards.rs`,
      prend le booléen en **paramètre**, et porte les quatre cas
      (`ReadyLabelAdd` × {true, false}, `IssueCreate` × {true, false}).
- [ ] `grep -n 'is_webhook_fallthrough_turn' crates/mika-agent/src/skills/builtin_handlers.rs`
      ne montre plus de `return Ok(())` conditionné à ce booléen dans
      `validate_fallthrough_work_creation` : le terme est passé au prédicat.
- [ ] L'argv exact de l'incident — `["issue", "edit", "2958", "--add-label", "ready"]` —
      est refusé avec `is_webhook_fallthrough_turn = false`, attesté par un test nommant
      le ticket.
- [ ] **Contrôle négatif d'AC2 dans le même appel de test** : la même fixture avec `ready`
      déjà porté par le ticket (pose hors moteur) laisse passer le dispatch forcé de
      `validate_forced_dispatch_consent`, verdict `allowed_consented`.
- [ ] `IssueCreate` est toujours autorisé hors tour Fallthrough — non-régression de
      mika#2573, attestée par un test qui rougit si l'asymétrie est perdue.
- [ ] Le `reason` du refus n'affirme plus « Webhook Fallthrough turn » sur la branche
      `ReadyLabelAdd`, et son `remedy` ne contient ni `--add-label`, ni `remove`, ni
      `add` comme gabarit de geste (assertion littérale dans le test).
- [ ] Le `"recovery"` de `validate_forced_dispatch_consent` ne prescrit plus la pose de
      `ready` ; il dit que la promotion appartient à l'opérateur. La branche `mika ask
      --agent mika-dev "groom …"` est conservée. Test d'assertion sur la chaîne.
- [ ] Le détecteur de `canonical_tokens.rs` est armé, son allowlist est **vide**, son
      contrôle de bonne foi rougit sur une prescription plantée, et son assertion de
      population non vide est présente.
- [ ] **Preuve rouge→vert** dans le corps de la PR : sortie du détecteur au commit 1
      (rouge, nommant le fichier et la ligne) et au commit 2 (vert).
- [ ] `cargo test -p mika-agent` vert, en particulier
      `mod mika2772_consentement_force` (`executor.rs:17037+`) et les tests mika#2573 de
      `guards.rs:7336-7505` — aucun n'est affaibli.
- [ ] `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills` et
      `scripts/verify-pipeline.sh origin/main` verts.
- [ ] `docs/operator/environment-variables.md` porte la nouvelle population de la garde,
      les deux motifs et leur lecture, la sonde d'AC5, et la note mika#2340 sur les deux
      instants d'activation.
- [ ] Le corps de la PR porte les lignes `Tracked in:` du § 7.

---

## 6. Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2995.

- **AC1.** Un `ready` posé par l'identité du moteur (l'App ou le jeton de mika-dev) ne
  vaut pas consentement pour la garde de mika#2772. Le plan nomme la lecture retenue :
  l'acteur du dernier événement `labeled ready` dans la timeline, une écriture de `ready`
  refusée au moteur, ou une autre voie. Il dit aussi pourquoi elle tient face à un agent
  qui veut lever la garde.
- **AC2.** Test dans le même appel, avec contrôle négatif. Le ticket porte `ready` posé
  par l'identité du moteur, et le dispatch forcé est refusé. Même fixture avec `ready`
  posé par un humain, et le dispatch passe.
- **AC3.** Le texte de remède du refus ne prescrit plus à l'agent un geste qu'il ne doit
  pas faire lui-même. Il dit que la promotion appartient à l'opérateur.
- **AC4.** Le plan recense tous les sites où le moteur lit `ready` comme consentement : le
  feeder `ready-label`, `stuck_ready_reconcile`, la porte de place mika#2084, et les
  autres. Pour chacun, il dit si le même trou existe, et ce vol le ferme ou le nomme.
- **AC5.** Sonde après déploiement : sur les 30 jours suivants, le compte d'événements
  `labeled ready` dont l'acteur est l'identité du moteur est écrit ici. La cible est 0, ou
  chaque cas est refusé par la garde.

### Comment chacun est tranché

**AC1** — § 1, voie B, avec son argument d'impossibilité et le refus argumenté de la
voie A. La voie retenue est **littéralement** la deuxième que l'AC énumère.

**AC2** — la paire vit dans un seul `#[test]`, et la **moitié « refusé » se lit sur l'axe
que la voie retenue a choisi**. C'est le point que l'architecte doit pouvoir vérifier sans
le reconstruire :

| moitié | site | fixture | attendu |
|---|---|---|---|
| positif | `validate_fallthrough_work_creation` (`builtin_handlers.rs`) | `ToolContext` avec `is_webhook_fallthrough_turn = false`, argv `["issue","edit","2958","--add-label","ready"]` — l'identité du moteur est **ce qui exécute l'appel**, c'est-à-dire la définition de la pose par le moteur | `Err(ToolOutput)`, `error == "fallthrough_work_creation_refused"`, `action == "ready_label_add"` |
| négatif | `validate_forced_dispatch_consent` (`executor.rs`), fixture `pilot_input("dev-groom", INCIDENT_SUBJECT, "task-2772")` | labels du ticket portant `ready`, posé hors moteur (le seul cas restant une fois l'écriture fermée) | `ForcedDispatchConsent::Allowed`, verdict `allowed_consented` |

Sous la voie B, « le ticket porte un `ready` posé par le moteur » n'est plus un état
atteignable : c'est **ce que le correctif rend impossible**. Le test positif atteste donc
le refus **de l'acte de pose**, et non le refus d'un état que plus rien ne peut produire —
un test qui fabriquerait cet état pour en observer le refus attesterait la voie A, celle
qui n'a pas été prise. Le contrôle négatif, lui, est inchangé dans sa forme : même
fixture, `ready` d'origine humaine, le dispatch passe. La paire sépare bien « la garde est
fermée au moteur » de « la garde est fermée à tout le monde ».

**AC3** — § 2.5, avec assertion littérale d'absence de gabarit.

**AC4** — § 2.3, neuf lignes, dont une négative explicite (la porte de place mika#2084, qui
lit la place et non le consentement) et trois `Tracked in:`.

**AC5** — la sonde, à écrire sur le ticket 30 jours après le déploiement :

```
# A — le compte que l'AC demande : poses de `ready` par l'identité du moteur
gh api -X GET 'repos/senara-solutions/mika/issues/events' --paginate \
  --jq '.[] | select(.event=="labeled" and .label.name=="ready") | .actor.login' \
  | sort | uniq -c | sort -rn

# B — les refus que la garde a produits, par motif et par population
sqlite3 ~/.mika/data/mika.db \
  "SELECT after_value, detail, COUNT(*) FROM audit_events
    WHERE tool_name='fallthrough_work_creation'
      AND created_at >= '<J+0>' GROUP BY 1, 2;"

# C — la ligne de journal, qui porte la dimension que B ne porte pas
grep -c 'fallthrough_work_creation_blocked' <journaux> ; \
  grep 'fallthrough_work_creation_blocked' <journaux> | grep -c 'turn_is_fallthrough=false'
```

Lecture, et ses deux pièges :

- La **cible est 0 en A** pour les logins machine (`mika-platform-dev`, l'App), **ou**
  chaque occurrence est appariée à une ligne de B/C. Un login machine en A **sans** ligne
  appariée est le signal d'une voie d'écriture non recensée : c'est un ticket, pas un
  réglage.
- Les poses par `apply_ready` (cascade de milestone, phases d'`auto_pull`) apparaissent
  en A sous l'identité machine et sont **légitimes** : le correctif ne les touche pas.
  Elles doivent donc être **nommées à part** dans le compte écrit sur le ticket, et non
  soustraites — deux populations sous un nom ne sont pas soustractibles (mika#2205).
- **Une absence de ligne en B ou C ne vaut pas une absence du phénomène** (mika#2340) :
  elle peut signifier un binaire antérieur au vol. La sonde se lit après avoir vérifié le
  sha de `~/.mika/skills/.manifest-writer`, et cette vérification s'écrit avec le compte.

---

## 7. Fire-Disposition

Le vol livre des détecteurs — le scan de `canonical_tokens.rs` (§ 2.6), les tests de
population de `guards.rs` et la paire d'AC2. La section est donc requise.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est livrée vide.**

Le détecteur de prescription a **une** violation existante dans son périmètre :
`skills/bundled/self-dev-callback/system_prompt.md:25`, mesurée par
`grep -rl -- 'add-label ready' skills/bundled/` → un seul fichier. Elle est **corrigée
dans le même vol**, au commit 2, juste après l'armement du détecteur au commit 1. Il ne
reste donc aucune violation à excepter, et l'allowlist est une constante vide — dans
l'idiome déjà en place sur ce dépôt (`mika2323_no_gate_predicate_reads_the_actor`,
`mika2425_identity_context_history_has_a_single_reader`).

Détail d'implémentation, parce qu'une allowlist vide a deux façons de mentir :

- **Assertion auto-nettoyante** — un test sœur affirme que la constante est vide
  (`assert!(ALLOWLIST.is_empty())` avec un message nommant mika#2995), de sorte qu'une
  entrée ajoutée un jour doit être retirée dès que sa violation disparaît : le geste qui
  rend l'exception inutile est le geste qui la fait expirer.
- **Assertion de population non vide** — le scan affirme qu'il a lu au moins un fichier.
  Un scan dont le `glob` ne résout rien passe vert par construction, et c'est la façon
  dont un détecteur de ce dépôt a déjà menti (classe mika#2578 / mika#2752 : une garde
  que le pipeline ne fait jamais tourner ne refuse rien).
- **Contrôle de bonne foi** — le scan est exécuté contre une fixture plantant la
  prescription, et le test échoue s'il ne rougit pas. C'est la moitié qui sépare « aucune
  violation » de « aucune détection ».

Les variantes `generated/` de `self-dev-*` sont **hors du périmètre de ce ticket par son
propre corps** et ne sont pas dans l'arbre de ce dépôt (`find skills/bundled -type d -name
generated` → vide) : elles ne sont donc **pas** une exception d'allowlist, elles sont hors
population. Elles font l'objet d'un `Tracked in:`.

**Tracked in:**

- `Tracked in: senara-solutions/mika#2995` — suivi à ouvrir : vérification une par une des
  variantes `generated/` de `self-dev-*`, hors périmètre par le corps du ticket.
- `Tracked in: senara-solutions/mika#2995` — suivi à ouvrir : le cas non inerte du
  callback, un groom lancé par `mika ask "groom mika issue#N"` sur un ticket sans `ready`
  (mika#2484) — choix d'une voie de reprise gouvernée ou d'une escalade opérateur.
- `Tracked in: senara-solutions/mika#2995` — suivi à ouvrir : fusion des trois prédicats
  `ready` dupliqués (`milestone_manager/pool.rs:1185`,
  `server/draft_pr_opened_handler.rs:86`, `webhook_dispatch.rs:599`) vers un lecteur
  unique, classe mika#2158.

> Les trois suivis sont référencés sur mika#2995 parce que leurs numéros n'existeront
> qu'à l'ouverture ; le corps de la PR portera les références réelles, et le milestone #33
> étant sous gel, leur ouverture est une décision opérateur.

---

## 8. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/evidence/guards.rs` — `work_creation_is_refused` + doc | 55 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` — câblage, textes par motif, champ de journal | 40 |
| `crates/mika-agent/src/skills/executor.rs` — réécriture du `"recovery"` (AC3) | 30 |
| `skills/bundled/self-dev-callback/system_prompt.md` — retrait de la prescription | 15 |
| détecteur + contrôle de bonne foi (`crates/mika-agent/src/canonical_tokens.rs`) | 85 |
| tests de population (`guards.rs`) | 60 |
| tests AC2, paire positif/négatif (`builtin_handlers.rs`) | 90 |
| test AC3 (`executor.rs`) | 35 |

Total estimé : 410 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 410 lignes contre une borne par vol de 429 et
un seuil par PR de 1 000 : la plus serrée (429) mord, et elle est tenue avec 19 lignes de
marge. La marge est mince, et c'est assumé pour une raison structurelle : **le vol n'est
pas découpable sans laisser un trou de sûreté ouvert.** Le prédicat sans le câblage ne
refuse rien ; le câblage sans le retrait de la prescription du § 2.4 fait refuser un geste
que le prompt servi continue de prescrire ; et le détecteur sans la correction laisse la
branche rouge. Les cinq commits du § 4 sont les cinq faces d'une même fermeture.

Deux faits bornent le risque de dérive de l'estimation. D'abord, **aucune ligne nouvelle de
logique** : le détecteur d'argv existe (mika#2573), la fixture de la garde existe
(`pilot_input("dev-groom", INCIDENT_SUBJECT, "task-2772")`), le `repo_root()` du scan
existe (`canonical_tokens.rs:63`), et les trois idiomes du § 7 ont chacun au moins deux
précédents dans l'arbre. Ensuite, **le poste le plus lourd est un scan de 85 lignes dont la
forme est copiable** sur `mika2323_no_gate_predicate_reads_the_actor` et son contrôle.

Si la mesure réelle dépassait la borne en cours de vol, la coupe est déjà nommée : le § 4
porte un plancher (commits 1–2, le détecteur armé et sa violation corrigée) qui est
mergeable seul et laisse la branche verte, et les commits 3–5 deviendraient le vol 2 avec
un `Tracked in:`. Ce n'est pas le plan — c'est la sortie de secours, et elle ne clôt pas
le ticket, puisque le plancher est de la prose et que le ticket exige l'impossibilité.
