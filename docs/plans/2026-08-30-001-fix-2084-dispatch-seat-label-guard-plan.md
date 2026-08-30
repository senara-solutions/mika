---
title: "fix(dispatch): lire l'étiquette de siège `dispatch:*` dans le chemin de dispatch"
type: fix
status: active
review: three-reviewer pass 2026-08-30 — seven findings applied
date: 2026-08-30
origin: senara-solutions/mika#2084
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
---

# fix(dispatch): lire l'étiquette de siège `dispatch:*` dans le chemin de dispatch

## Le POURQUOI

Un ticket peut porter `dispatch:ssc` ou `dispatch:mpc` — l'étiquette dit **quel dispatcher
le prend**. L'étiquette existe, elle est correcte, et **rien dans le code ne la lit**.
Vérifié : `grep -rni "\bssc\b|\bmpc\b|seat" crates/` ne rend que des commentaires de prose
(`crates/mika-agent/src/calibration/roles/mika_orchestrator.rs:3`,
`crates/mika-agent/src/milestone_manager/assessor.rs:3`). Aucun prédicat, aucune constante,
aucun appel. La notion de siège n'existe pas dans le moteur.

Conséquence mesurée le 2026-08-30 : mika#2055 portait `dispatch:ssc` depuis 06:53Z, SSC avait
ouvert la PR#2082 à 07:17 sur `bug/2055/sweep-printf-echo-grep-q-under-`, et à 07:43:21 la
boucle a créé une tâche `mika-dev` de plus sur le même ticket
(`70d30333-5999-417f-9166-29e19fdd954d`, `in_progress`), plus une seconde en file
(`7c820555-a4ae…`). Deux écrivains sur une branche. Les deux tâches ont été annulées à la main.

Une consigne de prompt ne suffira pas : ce dépôt a déjà mesuré que l'application par prompt ne
tient pas au niveau du substrat. La garde va dans le chemin de dispatch, et le refus se voit.

## Le fait qui décide de la conception

**La collision n'est pas passée par le chemin `ready-label`.** La tâche créée à 07:43:21 porte
`source: self_dev`, `trigger: manual`, et son libellé est
`CI fix: mika#2082 SIGPIPE lint — 2 occurrences in test-dispatch-lib.sh (issue mika#2055)`.
C'est un dispatch de correction CI, pas un événement `issues.labeled`.

Donc une garde posée **uniquement** dans `crates/mika-agent/src/server/ready_label_handler.rs`
n'aurait **pas** empêché l'incident que ce ticket décrit. La surface porteuse est la frontière
d'outil dans `crates/mika-agent/src/skills/executor.rs`, qui voit *tous* les chemins de
dispatch quelle que soit leur origine. C'est exactement le raisonnement déjà écrit en
`crates/mika-agent/src/skills/executor.rs:1064-1067` pour mika#2046 :

> This is the load-bearing layer. `run_claude_pilot` spawns a subprocess and creates a
> worktree, so a guard that only fires after the tool has run detects the violation without
> preventing it. […] The pre-LLM ready-label handler refuses the webhook path; this refuses
> every path, whatever originated the turn.

Le plan reprend cette architecture à trois couches, dans cet ordre d'importance.

## Le précédent qu'on imite : mika#2046

L'allowlist de dépôts dispatchables est la **même forme de garde**, déjà en place et validée.
On en copie la structure plutôt que d'en inventer une autre :

| Élément | mika#2046 (dépôt) | mika#2084 (siège) |
|---|---|---|
| Prédicats purs + constantes | `crates/mika-agent/src/webhook_dispatch.rs:103-143` (`DISPATCHABLE_REPOS`, `is_dispatchable_repo`, `dispatchable_repos_display`) | même fichier, nouvelle section « siège » |
| Garde pré-LLM (chemin webhook) | `crates/mika-agent/src/server/ready_label_handler.rs:125` | même fonction, après le fetch |
| Garde frontière d'outil (tous chemins) | `crates/mika-agent/src/skills/executor.rs:1072-1073` | même bloc, après celui-ci |
| Forme du refus | `warn!(event=…)` + `db.log_audit_event(…, "dispatch_refused", …)` + `VerdictAction::Handled` | identique |
| Test d'anti-vacuité | `loop_repos_are_not_caught_by_the_allowlist_gate` (`ready_label_handler.rs:886`) | obligatoire, cf. AC4 |

Le refus dans le handler retourne `Handled`, **jamais `Passthrough`** — le commentaire
`ready_label_handler.rs:118-124` explique pourquoi : `Passthrough` laisse `req.text` sur le
marqueur ready-label, que le guard `webhook_ready_label_dispatch` re-sollicite auprès du LLM
jusqu'à ce qu'il dispatche le ticket que la garde venait de refuser.

## Décisions structurantes

### D1 — Le siège de la boucle s'appelle `loop`, et il est tenu à la main

`dispatch:ssc` et `dispatch:mpc` désignent des sièges Claude Code interactifs. La boucle
autonome n'est **ni l'un ni l'autre** : c'est un troisième siège, qu'on nomme `loop`.
Conséquence directe et voulue — **tout ticket portant `dispatch:ssc` ou `dispatch:mpc` est
refusé par la boucle**, ce qui est précisément le comportement que l'incident réclame.

La liste des sièges connus est une constante tenue à la main, avec le même raisonnement que
`DISPATCHABLE_REPOS` (`webhook_dispatch.rs:103-142`) : *ce qui existe* et *ce qui est permis*
sont deux questions différentes, et seule la seconde est la politique. Une liste dérivée
(« tous les sièges qu'on a vus passer ») ferait de l'observation une autorisation.

### D2 — Fail-closed sur l'étiquette présente mais non résolue ; fail-open sur l'information absente

Ce sont deux cas distincts et ils ne se tranchent pas pareil.

- **Étiquette présente, siège non résolu** (`dispatch:zorglub`, `dispatch:`, ou **plusieurs**
  étiquettes `dispatch:*` sur la même issue) → **refus** (AC2). Un siège qu'on ne sait pas
  identifier n'est pas une autorisation. L'ambiguïté multi-étiquettes tombe dans le même sac :
  deux sièges revendiqués, aucun ne l'emporte.
- **Labels non récupérables** (l'appel `gh` échoue, pas de token GitHub configuré) → **on
  laisse passer**, avec un `warn!` distinct. Ce n'est pas une étiquette inconnue, c'est une
  absence d'information.

**Pourquoi ce second choix, explicitement.** Fail-closed sur l'échec de fetch transformerait
chaque hoquet réseau, chaque expiration de token, chaque limite d'API en arrêt complet de la
boucle. Le ticket nomme ce risque comme le plus grave du correctif : *« c'est un correctif qui
peut casser plus que le défaut qu'il répare »*. AC3 prime. Le risque résiduel est nommé :
pendant une panne `gh`, la garde de siège ne protège pas — mais la boucle continue de tourner,
et les deux autres couches (`auto_pull`, `ready_label_handler`) restent en place. Le `warn!`
rend la fenêtre comptable.

### D3 — La garde vit aussi dans `auto_pull.rs`, parce qu'elle n'y coûte rien

`crates/mika-agent/src/auto_pull.rs` est l'autre moitié du chemin : il **sélectionne** les
tickets et **pose** le label `ready` (`gh_apply_label(…, "ready")` en `auto_pull.rs:1280`,
`:1400`, `:1664`) — c'est-à-dire qu'il fabrique l'entrée du chemin de dispatch. Poser `ready`
sur un ticket d'un autre siège, c'est la collision en germe.

Le point qui tranche : **auto_pull a déjà les labels en mémoire**. Il les lit déjà
(`auto_pull.rs:347`, `:1164`, `:1483` — `issue.labels.iter().any(|l| l.name == "ready")`). La
garde y coûte donc **zéro aller-retour réseau**. Ne pas la mettre là serait laisser la boucle
étiqueter un ticket qu'elle se fera refuser trois couches plus loin — un cycle bruyant,
`ready` posé et jamais consommé, exactement le genre de boucle sourde que le milestone
« Substrat de boucle » existe pour éliminer.

Verdict choisi : `StuckReadyVerdict::Skip { reason: "seat_owned_by_other" }`, pas `Abandon`.
Le ticket n'est pas abandonné — il appartient simplement à quelqu'un d'autre, et l'opérateur
peut retirer l'étiquette à tout moment.

### D4 — Un seul parse d'issue, pas deux

La frontière d'outil doit connaître **le numéro d'issue**, pas seulement le dépôt.
`parse_repo_ref_from_dispatch_prompt` (`webhook_dispatch.rs:167`) rend déjà le `repo_ref` en
jetant le numéro. On ajoute `parse_issue_ref_from_dispatch_prompt` qui rend
`Option<(&str, u64)>`, et on **réimplémente** `parse_repo_ref_from_dispatch_prompt` par-dessus.
Deux parses indépendants de la même chaîne, c'est la dérive dont
`webhook_dispatch.rs:169-190` documente déjà le coût (« This must never be stricter than the
shell »). Une seule source de vérité.

## Requirements Trace

- **R1 (AC1)** — Le chemin de dispatch refuse de créer une tâche pour une issue portant
  `dispatch:<siège>` ≠ siège courant. Refus explicite nommant le numéro d'issue, l'étiquette
  trouvée et le siège courant.
- **R2 (AC2)** — Fail-closed sur l'inconnu : `dispatch:<inconnu>`, `dispatch:` vide, ou
  plusieurs `dispatch:*` → refus. Cf. D2.
- **R3 (AC3)** — Aucune étiquette `dispatch:*` → comportement **strictement** inchangé. Aucun
  chemin nouveau, aucun appel supplémentaire, aucun refus. Cf. D2 pour le cas « labels
  non récupérables ».
- **R4 (AC4)** — Test d'anti-vacuité **dans les deux sens** : refus pour un autre siège **et**
  passage pour une issue non étiquetée. Le second est obligatoire ; sans lui, « refuse tout »
  satisfait le premier.
- **R5 (AC5)** — Trace exploitable : `warn!(event = …)` structuré + `log_audit_event` avec
  action `dispatch_refused`, nommant issue, étiquette et siège courant.

## Scope Boundaries

**Dans le périmètre**
- `crates/mika-agent/src/webhook_dispatch.rs` — constantes de siège, `classify_dispatch_seat`,
  `parse_issue_ref_from_dispatch_prompt`, tests unitaires purs.
- `crates/mika-agent/src/server/ready_label_handler.rs` — `fetch_issue_body` élargi aux labels,
  garde + pré-digest de refus.
- `crates/mika-agent/src/skills/executor.rs` — garde frontière d'outil (**la couche porteuse**).
- `crates/mika-agent/src/auto_pull.rs` — filtre de sélection (coût zéro, cf. D3).
- Tests : unitaires dans les modules touchés + un test d'intégration.

**Hors périmètre**
- La création et la synchronisation des étiquettes `dispatch:*` elles-mêmes (le ticket l'exclut).
- Toute politique de notification (retrait de label, message opérateur) — mika#2046 a
  délibérément refusé d'élargir sa garde jusque-là (`ready_label_handler.rs:551-559`) ; on
  tient la même ligne.

### Suivi à ouvrir, pas à implémenter ici

`.github/labels.yml` ne contient **aucune** entrée `dispatch:*` (vérifié par grep), alors que
`.github/workflows/labels.yml` lance `EndBug/label-sync` avec `delete-other-labels: true`. Les
étiquettes de siège disparaîtront à la prochaine synchro, et les tickets perdront leur marqueur
sans que rien ne le dise — la garde de ce plan deviendrait alors silencieusement inopérante.
**À ficher comme ticket séparé** au moment de l'ouverture de la PR, et à mentionner dans le
corps de la PR.

## Implementation Units

### U1 — Prédicats de siège purs (`webhook_dispatch.rs`)

Nouvelle section, calquée sur celle de `DISPATCHABLE_REPOS`, avec le même commentaire
`// DOCTRINE: pre-classifier structural gate` que portent `is_dispatchable_repo`
(`webhook_dispatch.rs:129-136`) et `is_unauthorized_webhook_dispatch`
(`webhook_dispatch.rs:29-42`) — quel siège possède un ticket est un fait structural lu sur
l'étiquette, pas un jugement demandé au classifieur.

```rust
pub(crate) const DISPATCH_SEAT_LABEL_PREFIX: &str = "dispatch:";
pub(crate) const CURRENT_DISPATCH_SEAT: &str = "loop";
pub(crate) const KNOWN_DISPATCH_SEATS: &[&str] = &["loop", "ssc", "mpc"];

pub(crate) enum SeatVerdict {
    NoSeatLabel,                                  // AC3 — chemin inchangé
    OwnedByCurrentSeat { label: String },
    OwnedByOtherSeat { label: String, seat: String },
    Unresolvable { label: String, why: &'static str },  // AC2 — fail-closed
}

pub(crate) fn classify_dispatch_seat(labels: &[String]) -> SeatVerdict;
pub(crate) fn seat_verdict_refuses(v: &SeatVerdict) -> bool;
pub(crate) fn known_dispatch_seats_display() -> String;
```

Règles de `classify_dispatch_seat`, dans l'ordre :
1. Collecte les labels dont le nom (minusculisé, trimé) commence par `dispatch:`.
2. Zéro → `NoSeatLabel`. **Sortie la plus fréquente et la plus load-bearing** (AC3).
3. Plus d'un → `Unresolvable { why: "multiple_seat_labels" }`.
4. Suffixe vide → `Unresolvable { why: "empty_seat" }`.
5. Suffixe absent de `KNOWN_DISPATCH_SEATS` → `Unresolvable { why: "unknown_seat" }`.
6. Suffixe == `CURRENT_DISPATCH_SEAT` → `OwnedByCurrentSeat`.
7. Sinon → `OwnedByOtherSeat`.

Piège à ne pas rater : le préfixe est `dispatch:` **exactement**. Un label nommé `dispatched`
ou `dispatch-ready` ne doit pas entrer dans la garde — c'est un test explicite (T8).

Ajoute aussi `parse_issue_ref_from_dispatch_prompt(&str) -> Option<(&str, u64)>` par D4, et
réécris `parse_repo_ref_from_dispatch_prompt` comme `…map(|(r, _)| r)`. Le parse de ligne
existant (`parse_repo_ref_line`, `webhook_dispatch.rs:174-190`) rend déjà le numéro en interne
et le jette — le changement est mécanique et ne doit **rien** changer à la sévérité du parse.

### U2 — Garde frontière d'outil (`skills/executor.rs`) — **la couche porteuse**

À placer **immédiatement après** le bloc mika#2046 (`executor.rs:1069-1091`), même forme :
extraction du prompt → classification → `record_dispatch_rejection` + `Err(json)`.

Séquence : `parse_issue_ref_from_dispatch_prompt(prompt)` → si `None`, ne rien faire (texte
libre, aucune issue à juger — même règle que l'allowlist, cf. `webhook_dispatch.rs:191-193`) →
sinon récupérer les labels via `gh issue view <n> --repo <owner/repo> --json labels` →
`classify_dispatch_seat`.

Décisions d'implémentation à respecter :
- **Ordre.** Cette garde fait de l'I/O ; elle passe donc **après** toutes les vérifications
  pures existantes, jamais avant. Le commentaire de mika#2046 (`executor.rs:1070-1071`) dit
  explicitement que son bloc est placé avant le fetch de tâche *parce qu'il est pur* — la
  logique inverse s'applique ici.
- **Token.** Vérifier comment `executor.rs` accède à un token GitHub (le handler passe
  `github_token: Option<&str>`). **Pas de token → laisser passer + `warn!`** (D2).
- **Échec de l'appel `gh` → laisser passer + `warn!`** (D2).
- Message de refus sur le modèle de `executor.rs:1078-1089` : dire que c'est structural, pas
  transitoire, et que réessayer ne le lèvera pas.

### U3 — Garde pré-LLM (`server/ready_label_handler.rs`)

`fetch_issue_body` (`ready_label_handler.rs:525-541`) passe de `--json body -q .body` à
`--json body,labels` et rend `(String, Vec<String>)`. Un seul appel `gh`, **aucun aller-retour
supplémentaire** — c'est déjà l'appel de l'étape 4.

La garde s'insère **après** l'étape 4 (fetch) et **avant** l'étape 7 (pré-création de la tâche,
`ready_label_handler.rs:~215`), ce qui satisfait AC1 (« refuse de créer une tâche »). Elle ne
peut pas être plus haut : elle a besoin des labels.

Refus : `warn!(event = "ready_label_seat_mismatch", …)` + `log_audit_event(…,
"ready_label_seat_mismatch", …, Some("dispatch_refused"), …)` + `VerdictAction::Handled {
pre_digest: format_seat_mismatch_pre_digest(…) }`.

Le pré-digest ouvre par `<ready_label_handler>` — **obligatoire**, pour la raison écrite en
`ready_label_handler.rs:546-549` : sinon il matche le trigger `webhook_ready_label_dispatch` et
le guard réclame le dispatch que ce refus existe pour empêcher.

Échec du fetch → `Passthrough`, exactement comme aujourd'hui (`ready_label_handler.rs:186-196`).
Aucun changement de ce comportement (AC3).

### U4 — Filtre de sélection (`auto_pull.rs`)

Nouveau bras dans `classify_stuck_ready_in_memory` (`auto_pull.rs:420-443`), **après** le
filtre A (`is_feeder_excluded`) et avant le filtre B (`PlanOwnership`) :

```rust
// Filtre A2 : le ticket appartient à un autre siège de dispatch (mika#2084).
// Les labels sont déjà en mémoire — zéro I/O.
```

Verdict : `StuckReadyVerdict::Skip { reason: "seat_owned_by_other" }` (cf. D3). `Skip`, jamais
`SkipAndResetBudget` ni `Abandon` : l'appartenance à un autre siège n'est ni un progrès ni un
abandon.

Auditer aussi les autres points de sélection qui posent `ready` (`auto_pull.rs:1280`, `:1400`,
`:1664`) : si l'un d'eux ne traverse pas `classify_stuck_ready_in_memory`, il lui faut le même
filtre. **Ne pas supposer que la classification est le seul chemin** — grep tous les callsites
de `gh_apply_label(…, "ready")` avant de déclarer U4 fini.

### U5 — Surface de test

Anti-vacuité **dans les deux sens** (AC4) : chaque cas de refus est apparié à un cas de
passage. Un test qui ne mesure que le refus est vert même si la garde refuse tout.

`crates/mika-agent/src/webhook_dispatch.rs` — `mod tests` :
- **T1** `no_seat_label_still_dispatches` — labels `["bug", "p1-important", "ready"]` →
  `NoSeatLabel`. **(AC3, AC4 positif — le test le plus important du lot.)**
- **T2** `current_seat_label_still_dispatches` — `["dispatch:loop"]` → `OwnedByCurrentSeat`.
  (AC4 positif)
- **T3** `other_seat_label_is_refused` — `["dispatch:ssc"]` et `["dispatch:mpc"]` →
  `OwnedByOtherSeat`. (AC1)
- **T4** `unknown_seat_label_is_refused` — `["dispatch:zorglub"]` → `Unresolvable`. (AC2)
- **T5** `empty_seat_label_is_refused` — `["dispatch:"]` → `Unresolvable`. (AC2)
- **T6** `multiple_seat_labels_are_refused` — `["dispatch:ssc", "dispatch:mpc"]` →
  `Unresolvable`. (AC2)
- **T7** `seat_label_is_case_insensitive` — `["Dispatch:SSC"]` → `OwnedByOtherSeat`.
- **T8** `labels_merely_starting_with_dispatch_are_not_seat_labels` — `["dispatched",
  "dispatch-ready"]` → `NoSeatLabel`. **(AC3 — faux positif qui arrêterait la boucle.)**
- **T9** `parse_issue_ref_agrees_with_parse_repo_ref` — sur la matrice de prompts existante,
  les deux parses concordent. (D4, anti-dérive)

`crates/mika-agent/src/server/ready_label_handler.rs` — `mod tests` :
- **T10** `seat_mismatch_pre_digest_names_issue_label_and_current_seat` — le pré-digest
  contient le numéro, l'étiquette trouvée, le siège courant, `DISPATCH REFUSED`, et ouvre par
  `<ready_label_handler>`. (AC1, AC5)

`crates/mika-agent/src/auto_pull.rs` — `mod tests` :
- **T11** `issue_owned_by_other_seat_is_skipped` → `Skip { reason: "seat_owned_by_other" }`. (AC1)
- **T12** `unlabelled_issue_remains_eligible` → verdict inchangé vs. avant le correctif.
  **(AC3, AC4 positif.)**

`crates/mika-agent/tests/eval/test_dispatch_seat_label_guard.rs` — nouveau, calqué sur
`crates/mika-agent/tests/eval/test_dispatch_task_has_open_pr_guard.rs` :
- **T13** un dispatch dont le prompt vise une issue d'un autre siège est rejeté à la frontière
  d'outil, `record_dispatch_rejection` écrit. (AC1, AC5, **couvre l'incident réel**)
- **T14** un dispatch dont le prompt vise une issue sans étiquette de siège passe la garde.
  **(AC3, AC4 positif.)**

## Verification Contract

1. `cargo test -p mika-agent webhook_dispatch` — T1..T9 verts.
2. `cargo test -p mika-agent ready_label` — T10 vert, aucun test 2046 cassé.
3. `cargo test -p mika-agent auto_pull` — T11, T12 verts.
4. `cargo test -p mika-agent --test eval` (ou la cible qui porte `tests/eval/`) — T13, T14 verts.
5. `cargo clippy --all-targets -- -D warnings` propre.
6. `cargo fmt --check` propre.
7. **Vérification anti-vacuité manuelle** : commenter le corps de la garde dans `executor.rs`
   et confirmer que T13 **rougit**. Un test de garde qui reste vert sans la garde ne mesure
   rien (cf. `feedback_verify_pipeline_passes_without_the_fix`). Rétablir ensuite.
8. **Vérification AC3 manuelle** : confirmer que la suite complète `cargo test -p mika-agent`
   passe sans régression — l'écrasante majorité des tickets n'a pas d'étiquette de siège, donc
   toute rupture d'AC3 se voit comme une cascade d'échecs ailleurs.

## Definition of Done

- [ ] U1..U5 implémentés, `cargo test -p mika-agent` vert, clippy et fmt propres.
- [ ] La garde frontière d'outil (`executor.rs`) est en place — c'est la seule qui couvre le
      chemin `source: self_dev, trigger: manual` de l'incident.
- [ ] Le point 7 du contrat de vérification a été exécuté : la garde retirée, T13 rougit.
- [ ] Le suivi `.github/labels.yml` est fiché comme ticket séparé et cité dans le corps de la PR.
- [ ] PR ouverte avec `Closes #2084` et `mika-platform-qa` ajouté comme relecteur.

## Acceptance criteria

- [ ] **AC1** — Le chemin de dispatch refuse de créer une tâche pour une issue portant une
      étiquette `dispatch:<siège>` qui n'est pas le siège courant. Le refus est explicite :
      numéro d'issue, étiquette trouvée, siège courant.
- [ ] **AC2** — Le refus est **fail-closed** : une étiquette `dispatch:*` inconnue ou non
      résolue refuse aussi, plutôt que de laisser passer. Un siège qu'on ne sait pas identifier
      n'est pas une autorisation.
- [ ] **AC3** — Une issue sans étiquette `dispatch:*` se comporte exactement comme aujourd'hui.
      Le correctif ne doit pas transformer l'absence d'étiquette en refus — ce serait arrêter
      la boucle.
- [ ] **AC4** — Un test anti-vacuité dans les deux sens : une issue étiquetée pour un autre
      siège est refusée, **et** une issue non étiquetée est toujours dispatchée. Sans le
      second, « refuse tout » satisferait le premier.
- [ ] **AC5** — Le refus laisse une trace exploitable (événement ou ligne de journal nommant
      l'issue et le siège), pour qu'une collision évitée soit comptable au lieu d'être
      invisible.

## Addendum — ce que la revue a changé (2026-08-30)

Trois relecteurs. AC3 déclaré propre sur les quatre axes cherchés, parse partagé
strictement neutre, audit correct, pré-digest sûr sur les deux surfaces de garde.
Sept correctifs appliqués, dont trois qui invalident une décision du plan ci-dessus.

**Le plan avait tort sur le sujet du siège (U2).** Il désignait `reference_url`
comme source autoritaire. La revue a montré que `dispatch-lib.sh` ne lit jamais
`reference_url` : il lit `.prompt` (`dispatch-lib.sh:769`), en dérive le dépôt et
le numéro (`:1076-1080`), la branche (`:1131`) et le worktree (`:1176`). C'est
donc le prompt qui détermine où un second écrivain atterrirait. Une tâche
référençant l'issue A dispatchée avec `{"prompt": "mika#B"}` passait la garde.
**Les deux sujets sont désormais interrogés, et un refus de l'un refuse.**

**`gh issue view <n>` résout les numéros de PR.** Mesuré : `gh issue view 2082`
rend `{"labels":[]}` avec code 0 alors que 2082 est une PR. La garde ne
tombait donc pas en fail-open bruyant — elle rendait un verdict `NoSeatLabel`
confiant sur le mauvais objet, précisément sur la paire issue/PR de l'incident.
Passage au client REST borné (`github_graphql`, timeout 10 s), qui expose le
discriminant `pull_request` et supprime au passage un sous-processus `gh` sans
timeout du chemin chaud du dispatch.

**Le plan sous-estimait le nombre de sites qui posent `ready`.** U4 en comptait
trois, tous dans `auto_pull.rs`, et l'hypothèse « ils passent tous par
`is_feeder_excluded` » s'est vérifiée. Mais un **quatrième** existe hors de ce
fichier : `server/milestone_context_handler.rs:329`, la cascade de phase de
milestone, qui étiquette `ready` tous les tickets de la phase suivante sans lire
un seul label. Un `ready` posé là sur un ticket d'un autre siège serait refusé en
aval et **resterait posé pour toujours** — rien ne nettoie un `ready` non
consommé. Site fermé.

Autres correctifs : la garde remonte au-dessus du garde de dispatch global (une
refus après la mise en file brûlait un wrapper à chaque rejeu de la machinerie
de re-arm mika#2045) ; le motif de skip distinguait mal un siège étranger d'un
siège irrésolu, comptant un typo `dispatch:zorglub` comme une collision évitée —
et un test gravait l'erreur ; les messages opérateur affirmaient « un autre siège
possède ce ticket » là où aucun siège n'était identifiable ; balise
`</ready_label_handler>` non fermée ; liste des rejets terminaux de
`validate_dispatch_readiness` passée de huit à neuf.

**Vérification anti-vacuité, mesurée sur le code final** : garde neutralisée →
9 tests rougissent ; garde élargie à un refus universel → 32 rougissent.

### Reste ouvert, hors périmètre de ce ticket

- `.github/labels.yml` ne déclare aucune étiquette `dispatch:*` et
  `.github/workflows/labels.yml:21` lance `label-sync` avec
  `delete-other-labels: true`. À la prochaine synchro, les étiquettes de siège
  disparaissent de toutes les issues et cette garde se désarme **en silence**.
  Ticket de suivi ouvert ; c'est le risque dominant restant.
- `dispatch:loop` n'existe sur aucun dépôt, donc la revendication explicite d'un
  ticket par la boucle est aujourd'hui impossible. Même ticket de suivi.
- `select_stuck_ready_candidates` (`auto_pull.rs:502-527`) ne porte aucun filtre
  propre ; il n'est protégé que parce que `ages_by_issue` n'est peuplé que pour
  les survivants. Correct aujourd'hui, fragile demain.
- Le chemin LLM `gh issue edit --add-label ready`
  (`skills/bundled/self-dev-callback/system_prompt.md:25`) n'est pas contrôlable
  structurellement ici ; il est rattrapé en aval par les gardes de dispatch.
