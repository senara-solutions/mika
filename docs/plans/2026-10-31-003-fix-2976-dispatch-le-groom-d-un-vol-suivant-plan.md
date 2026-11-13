# mika#2976 — vol 3 : le callback d'un arrêt de dispatch ne répare rien, et la parente ne survit pas à son dernier enfant

**Portée : AC5, et AC5 seule.** Les deux vols précédents ont fermé la porte de reprise
(AC1/AC2/AC3/AC6/AC7/AC8/AC9, mergés en `07e3d6ff0` puis `feef58d45`) et ont, chacun,
renvoyé AC5 explicitement : le § 10 du plan du vol 1 — *« autre fichier, autre couche,
autre producteur »* — puis le préambule du plan du vol 2, qui clôt sur
`Tracked in: senara-solutions/mika#2976 (vol 3 — AC5)`. Ce vol honore ce renvoi.

AC5 a deux moitiés, et elles ne vivent ni dans le même fichier ni dans la même couche :

| moitié | énoncé | couche |
|---|---|---|
| A | « le callback ne lance pas de `git rebase` ni de résolution de conflit » | prompt, `skills/bundled/self-dev-callback/system_prompt.md` |
| B | « la parente devient terminale, sans rester `in_progress` après que son dernier enfant est livré » | Rust, `crates/mika-agent/src/task_engine/dispatcher.rs` |

---

## 1. Ce que la mesure dit, et ce qu'elle ne peut pas dire d'ici

**Mesuré, et décisif pour la moitié A.** Les deux bannières d'arrêt ne portent pas le
préfixe `PIPELINE FAILURE:`. Le routage du callback de groom
(`self-dev-callback/system_prompt.md` l. 30–34) teste, dans cet ordre : launcher-dead,
`Outcome: ESCALATE`, `PIPELINE FAILURE:`, puis **« Otherwise → route to the "On failure"
handler below (Step 4.5) »**. Les deux bannières tombent donc dans le dernier
« Otherwise ». Step 4.5 s'intitule *« Diagnose and recover from failure »* et son point 3
dit *« Attempt recovery (max 1) »*. Le tour de callback mesuré le 2026-10-30 n'a pas
désobéi : **il a suivi le prompt**. Le défaut est un discriminateur absent, pas une
dérive du modèle — et c'est ce qui le rend réparable par la même forme que les cinq
discriminateurs déjà en place.

**Mesuré, et décisif pour la moitié B.** Les hooks `try_*` de `deliver_callback`
(`dispatcher.rs` l. 1486–1545) tournent **avant** le tour silencieux — le commentaire du
site le dit en autant de mots : *« persist it to the parent task BEFORE the silent agent
runs. This guarantees at minimum … even if the agent exhausts its step budget »*. Trois
d'entre eux appartiennent déjà à la famille « rendre la parente terminale sans dépendre
du LLM » : `try_promote_parent_on_retry_success`, `try_complete_parent_on_callback_success`
(mika#1162, *« without depending on the LLM to fire the transition »*) et
`try_resolve_parent_on_dispatch_refusal` (mika#2158 AC8). L'incident mesuré est
précisément un tour mort en cours de réparation (« max steps exhausted », HEAD détachée à
`a219d0a3`, `rebase-merge` ouvert à 1/7) : **la terminalité ne doit donc pas dépendre de la
survie du tour.** Un septième frère, posé au même site, l'obtient par construction.

**Non mesurable depuis un bac à sable de pilote, et cela change le dessin.** La question
« pourquoi la parente `19c11b09` n'a-t-elle pas été fauchée ? » demande la base de
production. Elle n'est pas joignable d'ici : `~/.mika/data/` ne contient que
`pilot-transcripts`, aucun `mika.db` n'est atteignable, et `which mika` ne rend rien.
Neuf prédicats de `find_orphaned_parent_tasks` (`db/tasks.rs` l. 2140–2191) peuvent
expliquer l'exclusion, et rien d'ici ne les sépare. **Ce vol ne touche donc ni
`find_orphaned_parent_tasks` ni `reap_orphaned_parent_tasks`** — *un verdict non mesuré
ne se code pas* (plan du vol 1, § 10). L'implémenteur non plus n'aura cette base ni ce
binaire : qu'il ne cherche pas à mesurer, il n'en a pas les moyens.

Ce que la moitié B obtient **sans** cette mesure : elle rend la parente terminale au
moment de la livraison, soit au plus tard 600 s avant que le faucheur n'en devienne la
question. Le faucheur reste le filet de sécurité générique ; ce hook est le producteur
nommé pour cette population. Les deux ne se contredisent pas — `update_task_failed` est
un UPDATE gardé sur l'état terminal, donc le second à passer ne fait rien et le dit
(`Ok(false)`).

---

## 2. La moitié A — un sixième discriminateur, et son prédicat

À insérer dans `skills/bundled/self-dev-callback/system_prompt.md` **après** le
discriminateur groom-escalation (l. 84–95) et **avant** `**Pipeline result
classification**` (l. 97). La place est porteuse : plus bas, les deux bannières
retomberaient dans le « Otherwise » de Step 4.5, c'est-à-dire exactement le défaut.

> **Dispatch-halt discriminator (mika#2976 — MANDATORY, BEFORE pipeline result
> classification).**
>
> **Predicate :** la **première ligne** de `RESULT` est `STATUS=REBASE_CONFLICT` ou
> `STATUS=BRANCH_HAS_UNMERGED_WORK`. `starts with` est correct ici, comme pour le
> discriminateur cancel et contrairement à containment-refusal : `dispatch-lib.sh` pose
> ces deux `RESULT` lui-même (l. 4298 et l. 6077, les **seuls** `RESULT="STATUS=` du
> fichier), bannière en première ligne, avant tout autre texte.
>
> **Why :** les deux bannières disent *le dispatch n'a pas eu lieu, et rien n'a été
> touché* — « Nothing was reset, nothing was removed, nothing was stashed » pour l'une,
> « Resolve manually before re-dispatching » pour l'autre. Ni l'une ni l'autre ne porte
> `PIPELINE FAILURE:`, donc sans cette branche elles atteignent Step 4.5 *« Diagnose and
> recover from failure »*, qui invite une réparation : mesuré le 2026-10-30, un
> `git rebase origin/main` puis un `git checkout --theirs` dans le worktree d'un vol
> précédent, un tour mort en cours de route, et trois gestes manuels pour nettoyer.
> Une réparation dans un worktree est hors pipeline par nature : le `RESULT` nomme déjà
> le remède et il est en amont, dans la porte de reprise.
>
> 1. **Ne tente aucune réparation** : pas de `git rebase`, pas de `git checkout`, pas de
>    `git reset`, pas de résolution de conflit, aucun outil shell sur un worktree. Le
>    worktree reste **tel que le dispatch l'a laissé**.
> 2. **Ne retente pas** — ni `run_claude_pilot_groom`, ni `run_claude_pilot`. La cause est
>    l'état de la branche, qu'un second dispatch retrouverait identique.
> 3. **N'incrémente pas** `pipeline_retry_count`.
> 4. **Ne touche à aucun label** (`ready`, `blocked`, `operator-review`) — même règle que
>    les trois branches au-dessus : `auto_pull` reprend le ticket tel quel.
> 5. `update_task_status(task_id, "blocked")` avec la métadonnée
>    `{"dispatch_halt": true, "halt_status": "<REBASE_CONFLICT|BRANCH_HAS_UNMERGED_WORK>"}`
>    — comptable à part de `operator_cancel` / `containment_refusal` / `launcher_dead` /
>    `groom_escalated` (mika#2131).
> 6. `send_message` avec le `RESULT` **verbatim**. Il porte la branche, le nombre de
>    commits, les fichiers en conflit ou les commits non contenus, et le remède : un
>    opérateur y lit la cause sans ouvrir le worktree.
> 7. Passe à l'étape 6. N'annonce jamais une PR, une reprise, ni « awaiting QA review ».

Deux précisions que l'implémenteur ne doit pas avoir à deviner :

- **L'état d'enfant est `blocked`, l'état de parente est `failed`, et ce n'est pas une
  incohérence.** L'enfant est bloqué sur une cause extérieure réparable (la branche) ;
  la parente, elle, a échoué à produire son livrable, et `failed` est ce que le faucheur
  écrit déjà pour la même population. Deux axes, deux verbes.
- **Le prédicat est une table close de deux valeurs, jamais `STATUS=` seul.** Le champ
  `STATUS=` du `RESULT` est un espace de noms partagé : `STATUS=CANCELLED_BY_OPERATOR` et
  `STATUS=CANCELLED_BY_SIGNAL` y vivent aussi (pré-écrits par `cancel_task_and_kill` et
  par le trap TERM, l. 3352), et ils ont leur propre chemin, terminal en `cancelled`.
  Un prédicat sur le préfixe nu les avalerait et rendrait une annulation opérateur
  indistinguable d'un arrêt de branche.

---

## 3. La moitié B — un septième frère au site de livraison

### 3.1 Le lecteur

Dans `dispatcher.rs`, à côté de `parse_auto_skip_reason` et `parse_no_pr_reason` :

```rust
/// Les deux bannières d'arrêt de `dispatch-lib.sh`, lues en première ligne.
///
/// Table CLOSE, jamais le préfixe `STATUS=` : voir § 2 du plan — les deux
/// `CANCELLED_BY_*` partagent ce champ et sont terminaux autrement.
fn parse_dispatch_halt_status(result: &str) -> Option<&'static str>
```

Il rend `"REBASE_CONFLICT"`, `"BRANCH_HAS_UNMERGED_WORK"`, ou `None`. Prédicat : la
première ligne, après `trim_end`, égale `STATUS=<X>`.

### 3.2 Le hook

```rust
async fn try_terminate_parent_on_dispatch_halt(db: &AsyncDatabase, task: &Task)
```

Même forme que `try_resolve_parent_on_dispatch_refusal`, best-effort, fire-and-forget :

1. `task.parent_task_id` sinon retour.
2. `task.result` non vide, et `parse_dispatch_halt_status` non `None`, sinon retour.
3. `db.get_task_unscoped(&parent_id)` avec la **même** portée que les frères :
   `trigger_type == "manual" && status == "in_progress" && source == Some("self_dev")`.
   Pas de filtre sur `dispatch_class` — la bannière est produite par
   `_set_up_worktree`, qui tourne pour les deux classes, et les deux ont la même issue.
4. `db.update_task_failed(&parent_id, &motif)` — **la même primitive que le faucheur**,
   UPDATE gardé sur l'état terminal, donc inoffensif en course avec lui ou avec un geste
   opérateur. `Ok(false)` ⇒ `debug!` et rien d'autre.
5. Sur `Ok(true)` : `log_audit_event` avec `tool_name = "dispatch_halt_resolver"`,
   `in_progress` → `failed`, plus un `info!` `event = "dispatch_halt_parent_terminated"`
   portant `parent_task_id`, `callback_task_id`, `halt_status`.

**Le motif est distinct, et c'est une décision (mika#2205).** `dispatch_halt_<status>`,
jamais `callback_no_pr_*` : le faucheur écrit déjà ce dernier pour cette population — les
deux `RESULT` portent `NO_PR: no_pr_on_branch`, que `parse_no_pr_reason` lit — et deux
producteurs écrivant les mêmes octets rendent incomptable « combien de parentes ce hook a
rendues terminales » contre « combien le faucheur a dû rattraper ». La soustractibilité
*est* l'instrument de la sonde d'AC4.

### 3.3 L'appel

Une ligne dans la chaîne de `deliver_callback`, **après**
`try_resolve_parent_on_dispatch_refusal` et **avant**
`try_dispatch_pilot_after_groom_success`. Après le refus parce que les deux prédicats
sont disjoints et que l'ordre y est sans effet ; avant le dispatcheur de pilote pour que
la parente soit déjà terminale si celui-ci lit son état.

### 3.4 Le coût, nommé

Rendre la parente terminale **avant** le tour prive ce tour de toute relance sous la même
parente. C'est voulu, et c'est déjà le contrat du point 2 du discriminateur. La reprise
passe par `auto_pull`, qui retrouve le ticket avec son label intact — même argument que
les trois discriminateurs voisins (*« `auto_pull` resumes the ticket as-is »*). Ce qui
disparaît est la parente zombie, pas le ticket.

---

## 4. Le registre des jetons canoniques — deux lignes dues

`parse_dispatch_halt_status` est un lecteur **strict** d'un jeton produit par
`dispatch-lib.sh`, donc classe B de mika#2201. Les deux bannières n'y sont pas déclarées
aujourd'hui (`grep REBASE_CONFLICT scripts/canonical-tokens.tsv` ne rend rien) ;
`canonical_tokens::tests::mika2201_every_match_site_is_declared` et
`canonical-tokens-survey.sh --check` l'exigeront, et une CI rouge est la façon dont
l'implémenteur l'apprendra s'il l'oublie. À ajouter, avec son bloc de commentaire sur le
modèle de celui des `CANCELLED_BY_*` :

```
REBASE_CONFLICT	B	crates/mika-agent/src/task_engine/dispatcher.rs::parse_dispatch_halt_status	exact:line-anchored
BRANCH_HAS_UNMERGED_WORK	B	crates/mika-agent/src/task_engine/dispatcher.rs::parse_dispatch_halt_status	exact:line-anchored
```

Le commentaire doit nommer le second lecteur, qui n'est pas du source et sort du
périmètre du lint : le prompt de `self-dev-callback`. Une bannière reformulée y ferait
retomber les deux arrêts dans Step 4.5 — le défaut d'origine, restauré en silence.

---

## 5. Les tests, et où s'arrête la garantie

### 5.1 La moitié B, attestée structurellement — `dispatcher.rs § tests`

Cinq cas, sur le harnais des frères (une base en mémoire, une parente, un enfant de
callback) :

1. `REBASE_CONFLICT` sur une parente `in_progress` self_dev manuelle ⇒ parente `failed`,
   motif `dispatch_halt_REBASE_CONFLICT`, événement d'audit écrit.
2. `BRANCH_HAS_UNMERGED_WORK` ⇒ même issue, motif distinct.
3. **Contrôle négatif de la table close** : `STATUS=CANCELLED_BY_OPERATOR` ⇒ parente
   **intacte** en `in_progress`. C'est ce cas qui sépare « le prédicat mord sur les deux
   arrêts » de « le prédicat mord sur tout `STATUS=` ».
4. **Contrôle négatif de portée** : une parente déjà `completed` ⇒ `Ok(false)`, aucun
   événement d'audit, rien d'écrit.
5. **Contrôle négatif de forme** : la bannière présente mais **pas** en première ligne
   (précédée d'un `Log path: …`, la forme de containment-refusal) ⇒ pas de
   transition. Le prédicat est ancré, et ce test est ce qui le dit.

### 5.2 L'ordre dans le prompt, attesté — `dispatcher.rs § tests`

Un scan sur le prompt **embarqué**, par `include_str!("../../../skills/bundled/self-dev-callback/system_prompt.md")`,
sur l'idiome de `qa_reviewed_head.rs § conformité des prompts` (l. 860–870) et pour sa
raison : `include_str!` fige les octets au build, exactement comme `build.rs` les fige
dans `BUNDLED_SKILL_MANIFESTS` ; une édition de l'arbre est invisible aux deux jusqu'au
prochain `make deploy` (motif mika#2340). Le test asserte trois choses, chacune étant la
forme exacte du défaut :

- les deux bannières apparaissent dans le prompt ;
- leur première occurrence est **avant** celle de `Pipeline result classification` ;
- l'allowlist d'exception est **vide**, et pinée vide (§ 6).

### 5.3 La moitié A, attestée de ce qu'elle peut l'être — huitième cas du harnais de cohérence

`crates/mika-agent/tests/eval/test_self_dev_callback_engine_consistency.rs` porte déjà
sept cas dont l'objet est exactement celui-ci : *« each documented callback-handler branch
… prescribes tool calls that the engine accepts (or properly defers) — not hard-rejected
by guards »*. Huitième cas : un tour de callback dont le `RESULT` est le verbatim
`REBASE_CONFLICT` de la tâche `1f838a67`, le `MockLlmProvider` émettant le chemin prescrit
(`update_task_status` + `send_message`). Assertions : le moteur l'accepte — aucune garde
ne redemande un tour, le refus zéro-appel (#870) ne mord pas — et **aucun outil shell ni
git n'est appelé**, ce qui est la forme testable de « laisse le worktree tel que le
dispatch l'a laissé » : le chemin prescrit est complet sans toucher au worktree.

Contrôle négatif dans le même fichier : un `RESULT` de panne ordinaire, sans bannière,
continue d'atteindre Step 4.5 et peut donc tenter sa réparation. C'est lui qui sépare
« le discriminateur mord sur la bonne population » de « Step 4.5 a été désarmé en bloc ».

### 5.4 Où s'arrête la garantie, dit plutôt que supposé

**Un mock valide la forme du prompt, jamais l'obéissance du modèle** (doctrine mika#1190,
reprise au § `[context.summary]` de `crates/mika-agent/CLAUDE.md`). La moitié A reste donc
**prescriptive** : rien n'empêche structurellement un tour futur de lancer un `git rebase`.
Ce que ce vol garantit structurellement, c'est que **ce rebase ne coûte plus la parente** —
elle est déjà terminale quand le tour prend la main, et la relance n'est plus bloquée. Les
trois gestes manuels du 2026-10-30 (`rebase --abort`, retrait du worktree, `cancel` de la
parente) se réduisent aux deux premiers, qui ne bloquent rien.

Fermer la moitié A structurellement demanderait une garde EndTurn refusant un outil
mutant un worktree dans un tour de callback portant une bannière d'arrêt. **Ce vol ne la
livre pas**, et pas par oubli : c'est une garde sur le domaine entier des tours de
callback, dont la population — combien de tours de callback touchent légitimement un
worktree aujourd'hui ? — n'est pas mesurée, et une garde dont on ne connaît pas la
population qu'elle va mordre est la forme que mika#2309 condamne.
`Tracked in: senara-solutions/mika#2976 (suivi — garde EndTurn du worktree en tour de callback)`

---

## 6. Fire-Disposition

Ce plan livre des détecteurs : les cinq cas du § 5.1, le scan d'ordre du § 5.2, les deux
cas du § 5.3, et les deux lignes du registre canonique (§ 4) dont les deux gardes
existantes feront des détecteurs.

**Option retenue : (a) exception nommée en allowlist, allowlist livrée VIDE.**

Le détail d'implémentation, parce que « vide » n'est une disposition que si elle est
tenue :

- Le scan d'ordre du § 5.2 porte une `const ORDER_SCAN_ALLOWLIST: &[&str] = &[]`, et un
  test frère asserte `ORDER_SCAN_ALLOWLIST.is_empty()` avec, dans son message d'échec,
  l'instruction : *une entrée ici veut un ticket de suivi nommé et la date de son
  retrait*. C'est l'assertion auto-nettoyante : le geste qui rend une exception
  nécessaire est celui qui la fait expirer (forme déjà en place pour mika#2425 et
  mika#1833, *« allowlist shipped empty, pinned empty by a sibling test »*).
- Aucune violation existante à exempter : le prompt n'a qu'un site concerné et le même
  commit l'y rend conforme (§ 7). Le compte de départ est donc zéro, mesuré par le scan
  lui-même à son premier passage, et non supposé.
- Les détecteurs des § 5.1 et § 5.3 sont des tests neufs sur du code neuf : ils passent
  au vert dans le commit qui les introduit, sans exception d'aucune sorte.

Aucun détecteur n'est livré désarmé, et aucun n'escalade : l'option (b) et l'option (c)
sont écartées parce que rien n'oblige ici à les prendre — il n'y a pas de feu à éteindre.

---

## 7. Ordre de commit, et le plancher

Chaque commit passe `cargo test -p mika-agent`, `cargo clippy --all-targets`,
`cargo fmt --check`, `scripts/check-canonical-tokens.sh` et
`scripts/verify-pipeline.sh origin/main`. Aucun commit intermédiaire rouge.

1. **La moitié B, seule.** `parse_dispatch_halt_status`,
   `try_terminate_parent_on_dispatch_halt`, l'appel, les cinq tests du § 5.1 et les deux
   lignes du registre (§ 4) — ensemble, parce que le lecteur strict sans sa déclaration
   rougit la CI. Ce commit vaut par lui-même : la parente cesse d'être zombie même si le
   prompt n'est pas encore déployé. C'est la moitié qui ne dépend pas de `make deploy`.
2. **La moitié A.** Le discriminateur du § 2, le scan d'ordre du § 5.2 avec son allowlist
   vide, l'huitième cas du § 5.3 et son contrôle négatif — ensemble, parce que le scan
   rougirait sur un prompt pas encore modifié.
3. **La trace.** Complément à
   `docs/solutions/logic-errors/stale-base-conflicting-prs-no-self-heal-2026-04-23.md` :
   le second visage du défaut, l'asymétrie prescriptif/structurel du § 5.4, et le relevé
   de la sonde d'AC4. Hors du compte de taille.

**Ordre de déploiement, qui n'est pas l'ordre de commit.** La moitié B mord dès que le
binaire tourne ; la moitié A ne mord qu'après `make deploy` (rebuild → seed → read,
mika#2340). Entre les deux, un callback d'arrêt peut encore tenter une réparation — mais
sans laisser de parente zombie. La dégradation est dans le bon sens, et c'est pourquoi
B part en premier.

---

## 8. La sonde d'AC4 — ce que ce vol y ajoute

Le troisième relevé (2026-10-31, 09:38Z) tient : 0 `REBASE_CONFLICT` et 0 arrêt à tort sur
1 groom de vol suivant depuis le vol 2, 9 relevés restants. Ce vol n'y touche pas le
compte, il y ajoute **un terme lisible**. La limite nommée par le vol 2 — *« la ligne
`[dispatch-lib] branch_reuse_gate: …` n'est écrite dans aucun journal persistant »* —
reste entière ; mais `dispatch_halt_parent_terminated` et le `tool_name`
`dispatch_halt_resolver` sont, eux, en base, donc comptables. Le relevé suivant pourra
dire *combien d'arrêts de dispatch ont rendu leur parente terminale sans geste humain*,
ce que le vol 2 ne permettait pas de lire. L'**absence** de ces lignes signifie un binaire
antérieur à ce vol (motif mika#2340), jamais une absence d'arrêt.

---

## 9. Discipline de l'implémenteur — cinq pièges nommés

1. **Ne touche ni `find_orphaned_parent_tasks` ni `reap_orphaned_parent_tasks`.** Neuf
   prédicats peuvent expliquer la parente non fauchée du ticket, et aucune mesure d'ici ne
   les sépare (§ 1). Le doc-comment de `find_orphaned_parent_tasks` (l. 1989–2139) porte en
   outre un avertissement **Coupled pair** avec
   `find_completable_parent_tasks_on_pr_url` : tout changement de filtre doit être
   répliqué. Ce vol n'en change aucun.
2. **`REAPER_GRACE_SECONDS` n'est pas un levier.** Depuis mika#2980 la même constante
   borne aussi la fenêtre de vivacité du frère `completed`. La recalibrer déplace deux
   comportements, dont un que ce vol n'a pas mesuré.
3. **Le prédicat est une table close.** Deux valeurs, en première ligne. Ni `STATUS=`,
   ni `contains`, ni une alternance qui attraperait un `CANCELLED_BY_*` (§ 2, § 5.1 cas 3).
4. **La place du discriminateur dans le prompt est le correctif.** Déplacé sous
   `Pipeline result classification`, il redevient inopérant sans qu'aucune assertion de
   comportement ne rougisse : seul le scan du § 5.2 le voit. Ne le neutralise pas en
   « rangeant » le fichier.
5. **Le motif d'échec reste distinct de celui du faucheur.** `dispatch_halt_<status>`, pas
   `callback_no_pr_*`. Les deux `RESULT` portent `NO_PR: no_pr_on_branch` et
   `parse_no_pr_reason` le lit déjà : réutiliser ce motif ferait écrire aux deux
   producteurs les mêmes octets, et rendrait la sonde du § 8 illisible (mika#2205).

---

## Definition of Done

- [ ] `parse_dispatch_halt_status` et `try_terminate_parent_on_dispatch_halt` livrés dans
      `dispatcher.rs`, appelés depuis la chaîne de `deliver_callback` entre
      `try_resolve_parent_on_dispatch_refusal` et `try_dispatch_pilot_after_groom_success`.
- [ ] La parente d'un callback portant l'une des deux bannières passe `failed` avec un
      motif `dispatch_halt_<status>`, un événement d'audit `dispatch_halt_resolver` et un
      `info!` `dispatch_halt_parent_terminated`.
- [ ] Le discriminateur dispatch-halt est présent dans
      `skills/bundled/self-dev-callback/system_prompt.md`, placé avant
      `Pipeline result classification`, dans la forme des cinq discriminateurs existants
      (Predicate / Why / étapes numérotées).
- [ ] Les deux bannières sont déclarées classe B dans `scripts/canonical-tokens.tsv` avec
      leur site de match et leur tolérance.
- [ ] Les cinq tests du § 5.1 passent, dont les trois contrôles négatifs (table close,
      portée, ancrage de ligne).
- [ ] Le scan d'ordre du § 5.2 passe, allowlist vide et pinée vide.
- [ ] Le huitième cas du § 5.3 et son contrôle négatif passent dans
      `test_self_dev_callback_engine_consistency.rs`.
- [ ] `cargo test -p mika-agent`, `cargo clippy --all-targets`, `cargo fmt --check`,
      `scripts/check-canonical-tokens.sh`, `make verify-bundled-skills` et
      `scripts/verify-pipeline.sh origin/main` verts sur chaque commit.
- [ ] `docs/solutions/logic-errors/stale-base-conflicting-prs-no-self-heal-2026-04-23.md`
      complété du second visage et de l'asymétrie du § 5.4.
- [ ] Le corps de la PR porte `Code review: delegated to mika-qa (mika#2694)` et la ligne
      `Tracked in:` du § 5.4.

---

## Acceptance criteria

- **AC1.** Le groom d'un vol suivant, sur un ticket dont la PR précédente est mergée, ne reprend pas l'ancienne branche, qu'elle soit sur origin ou seulement en local. Il produit un plan sur une branche qui part de `main`.
- **AC2.** Test dans le même appel, avec contrôle négatif : (a) une branche locale dont la PR est mergée n'est pas reprise ; (b) une branche sur origin dont la PR est mergée n'est pas reprise ; (c) une branche dont la PR est ouverte **reste** reprise, ce qui est le cas nominal de reprise d'un vol en cours.
- **AC3.** Aucune perte : si l'ancienne branche porte des commits absents de `main` et sans PR mergée, le dispatch le dit et s'arrête, au lieu de les écraser.
- **AC4.** Sonde après déploiement : sur les 10 grooms suivants d'un vol 2 ou plus, le compte de `REBASE_CONFLICT` dus à une branche de vol mergé est écrit ici. La cible est 0.
- **AC5.** Le callback de `REBASE_CONFLICT` ne lance pas de `git rebase` ni de résolution de conflit dans un worktree. Il rend la main, et la parente devient terminale, sans rester `in_progress` après que son dernier enfant est livré. Test dans le même appel : un callback `REBASE_CONFLICT` laisse le worktree tel que le dispatch l'a laissé, et la parente passe à un état terminal.
- **AC6.** Test dans le même appel, sur une fixture **à plusieurs commits** (au moins le plan et un commit de code) squashée en **un seul** commit sur `main` : le verdict est `spent` et l'action `reset`. Contrôle négatif dans la même fixture : un commit ajouté après le squash, absent de `main`, donne `unmerged_work` et l'arrêt.
- **AC7.** Le cas mesuré : la branche `feat/2958/manager-phase-2-c-vol-2-le-manager` à `59010336a`, contre `origin/main` qui porte `fcb5148fd`, rend `spent`. La branche locale a été supprimée le 2026-10-31 sur GO de samidarko ; elle se retrouve par son sha.
- **AC8.** Un commit postérieur à la tête mergée de la dernière PR terminale de la branche est le vol courant, pas un travail laissé derrière : le verdict n'est pas `unmerged_work`. Test dans le même appel, avec contrôle négatif : une branche recréée sur `main` avec un seul commit de plan, PR précédente mergée, donne un verdict qui laisse partir dev-pilot ; la même branche avec, en plus, un commit d'avant le merge absent de `main` s'arrête toujours.
- **AC9.** Une ref de suivi `origin/<branche>` dont la branche n'existe plus sur origin ne fait pas buter la porte. Le plan dit si la porte la lit encore, et comment elle sait qu'elle est périmée.

**Couverture par vol.** AC1, AC2, AC3 : vol 1 (`07e3d6ff0`). AC6, AC7, AC8, AC9 et le
correctif de l'instrument d'AC4 : vol 2 (`feef58d45`). **AC5 : ce vol.** AC4 est une sonde
à relevés successifs, tenue à jour dans le corps du ticket ; son troisième relevé est
porté au § 8.

---

## Taille estimée

| livrable | lignes de code (hors docs/) |
|---|---|
| `skills/bundled/self-dev-callback/system_prompt.md` — discriminateur dispatch-halt | 18 |
| `dispatcher.rs` — `parse_dispatch_halt_status`, `try_terminate_parent_on_dispatch_halt`, l'appel (doc-comments compris) | 115 |
| `dispatcher.rs § tests` — cinq cas du § 5.1 | 130 |
| `dispatcher.rs § tests` — scan d'ordre du prompt + allowlist vide pinée | 35 |
| `test_self_dev_callback_engine_consistency.rs` — huitième cas + contrôle négatif | 85 |
| `scripts/canonical-tokens.tsv` — deux lignes + bloc de commentaire | 10 |

Total estimé : 393 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3
