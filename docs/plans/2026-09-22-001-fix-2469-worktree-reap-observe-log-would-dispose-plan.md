# mika#2469 — En mode observe, le reaper de worktrees dit ce qu'il *ferait*, pas ce qu'il a fait

**Ticket :** mika issue#2469
**Type :** fix (observabilité — substrat moteur, scan `worktree_reap`)
**Date :** 2026-09-22

---

## Problème

Le scan `worktree_reap` (mika#2420, `crates/mika-agent/src/worktree_reaper.rs`)
a deux dispositions : `armed` (défaut) supprime ; `observe`
(`MIKA_WORKTREE_REAP_DISPOSITION=observe`) mesure et journalise **sans rien
supprimer**. C'est le patron mika#2249 : *la détection est inconditionnelle,
seule la disposition est gardée.*

Le tir du 2026-09-21 16:10:04Z en `observe` a produit :

```
{"message":"worktree_reap: worktree de PR terminale retiré","event":"worktree_reaped",
 "worktree_path":".../fix-2184-task-engine-17-des-expirations-fautives/mika","disposition":"observe"}
```

et le worktree fix-2184 **existait toujours** après le tir. Le message est au
passé accompli, l'event est le nom d'une action, et seul le champ
`disposition:"observe"` — en fin de ligne — dit qu'il ne s'est rien passé. Un
lecteur du log conclut à un retrait qui n'a pas eu lieu. Le log est la surface
d'incident (classe mika#2041) ; le HALT 1 de la fiche CLAUDE.md du reaper
(*« lire `worktree_reaped` pour établir quel terme a lu vrai »*) se lit
justement là.

### M1 — Ce que la lecture du code établit : ce n'est pas *une* ligne, c'est *une constante*

`worktree_reaper.rs:1240-1254` :

```rust
info!(
    event = REAPED_TOOL,               // "worktree_reaped"
    …,
    disposition = cfg.disposition.as_str(),
    "worktree_reap: worktree de PR terminale retiré"
);
record_reaped(db, session_id, &candidate, &size, cfg.disposition, trace_id).await;
```

et `record_reaped` (`:1292-1340`) écrit `audit_events.tool_name = REAPED_TOOL`
avec `disposition=observe` **dans `reasoning`**. `REAPED_TOOL` (`:196`) est **une
seule constante qui sert d'`event` tracing et de `tool_name` d'audit**. Le
symptôme du ticket est donc présent sur **deux** surfaces, pas une :

| surface | ce qu'elle dit en observe | ce qui s'est passé |
|---|---|---|
| `$MIKA_SPIRIT_LOG_FILE`, `event=worktree_reaped`, message « retiré » | un retrait | rien |
| `audit_events`, `tool_name='worktree_reaped'` | un retrait | rien |

Et la doc contractuelle du second est **fausse en observe** : le commentaire de
`REAPED_TOOL` (`:190-195`) et `CLAUDE.md:1579` affirment que
`SELECT … WHERE tool_name = 'worktree_reaped'` est *« la liste exacte des
worktrees que la boucle a retirés — le garde-fou 3 du ticket »*. Un tick en
observe y ajoute des lignes qui ne sont pas des retraits. Le corps de #2469
nomme le log ; la lecture du code montre que corriger le log sans l'audit
laisserait la requête opérateur documentée mentir de la même façon, et que les
deux tiennent à la même constante.

### M2 — Le précédent maison, et il tranche la forme

Le faucheur de pilotes silencieux (`task_engine/engine.rs:2630-2760`, mika#2249 →
mika#2272) a résolu exactement cette question : son event et son `tool_name`
sont `pilot_silent_stall` — **le nom de la détection, pas de l'action** — et ce
qui a été fait est porté par deux champs, `disposition_armed` et
`transitioned`, avec un `after_value` qui *« dit ce qui s'est réellement passé,
pas ce qui était voulu »*. Le WARN dit : *« leaving the task in_progress rather
than claiming a disposition that did not happen »*. C'est la règle : **une ligne
ne revendique jamais une disposition qui n'a pas eu lieu.**

Ici, l'event existant `worktree_reaped` est déjà *le nom d'une action* — et il
est contractuel (SOLE WRITER, requête opérateur, HALT 1). On ne le neutralise
pas ; on le **réserve** à l'action, et on donne un nom propre au cas observe.
C'est ce que le corps du ticket propose (`worktree_reap_would_dispose`), et
c'est cohérent avec la famille existante (`worktree_reap_skipped`,
`worktree_reap_failed`, `worktree_reap_tick`, `worktree_reap_no_token`, …).

### M3 — Terminologie du corps : « enact » = `armed`

Le corps écrit *« réservés au mode `enact` où la suppression a réellement
lieu »*. Le mode s'appelle `armed` dans le code (`Disposition::Armed`,
`as_str() = "armed"`, `CLAUDE.md` : *`armed` (default) | `observe`*). Le
corps décrit le mode par son effet, pas par son identifiant ; l'intention
est univoque (le mode qui supprime). Aucune divergence de fond — le plan
emploie `armed` partout.

---

## Requirements

- R1. En disposition `observe`, la ligne INFO par worktree éligible ne porte
  **ni** l'event `worktree_reaped` **ni** un message au passé accompli ; elle
  nomme l'éligibilité et dit explicitement « non retiré ».
- R2. En disposition `observe`, la ligne d'audit par worktree éligible ne porte
  **pas** `tool_name = 'worktree_reaped'` ; elle porte un `tool_name` propre à
  l'observation.
- R3. `worktree_reaped` (event **et** `tool_name`) reste **réservé** à un retrait
  qui a eu lieu (`armed` et `removal.removed == true`). La requête opérateur
  `SELECT … WHERE tool_name = 'worktree_reaped'` redevient exactement ce que sa
  doc affirme.
- R4. Les deux surfaces (log, audit) sont nommées par **une seule source de
  vérité** par disposition : impossible de faire diverger l'event tracing du
  `tool_name` d'audit sans toucher un site unique.
- R5. Le champ `disposition` reste présent sur les deux lignes et
  `disposition=<x>` reste dans `reasoning` : la population « ce qui *serait*
  supprimé » (raison d'être du mode observe, D7 de mika#2420) reste requêtable
  et lisible, et les tests/rapports existants qui lisent `disposition=armed` ne
  bougent pas.
- R6. Le garde structurel SOLE WRITER (`mika2420_le_tool_name_daudit_a_un_seul_writer`)
  couvre le nouveau `tool_name` comme l'ancien : un seul module écrit l'un et
  l'autre.
- R7. Aucune valeur de réglage, aucun terme du prédicat T1–T7, aucun compteur
  du tick (`disposed`/`failed`/`refused`), aucune migration ne change. Le
  chemin `armed` produit **exactement** les mêmes lignes qu'aujourd'hui.
- R8. Les deux documents contractuels sont mis en accord : le doc de module de
  `worktree_reaper.rs` (§ « Trois leviers » et commentaire de `REAPED_TOOL`) et
  `CLAUDE.md` § reaper (lignes du levier `MIKA_WORKTREE_REAP_DISPOSITION`, des
  surfaces opérateur, et de la sonde post-déploiement « commencer en observe »).

---

## Décisions

### D1 — Réserver l'ancien nom, nommer le cas observe ; pas d'event neutre

Deux formes étaient possibles pour satisfaire R1–R3 :

- **(a)** un event **neutre** unique (`worktree_reap_eligible`) portant
  `removed: bool` — la forme `pilot_silent_stall` ;
- **(b)** **deux** events : `worktree_reaped` réservé au retrait effectif,
  `worktree_reap_would_dispose` pour l'observation.

**(b) est retenue.** `worktree_reaped` est contractuel sur trois surfaces qui
existent déjà (requête opérateur CLAUDE.md, HALT 1, garde SOLE WRITER) et il
*est* exact en `armed`. La forme (a) obligerait à réécrire ces trois surfaces
et à ajouter un filtre `removed = true` à la requête opérateur — un coût sans
gain, et un risque : une requête historique non mise à jour se mettrait à
compter des observations. Le précédent `pilot_silent_stall` a choisi (a) parce
que son nom était **déjà** celui d'une détection ; ici le nom existant est déjà
celui d'une action, et c'est la situation inverse. La leçon transportée n'est
pas la forme mais la règle : *ne jamais revendiquer une disposition qui n'a pas
eu lieu*.

Le nom `worktree_reap_would_dispose` est celui que le corps du ticket propose ;
il dit le conditionnel (« aurait retiré ») sans ambiguïté et suit le préfixe
`worktree_reap_` de la famille.

### D2 — Un seul site décide du triplet (event, tool_name, message)

Une fonction pure :

```rust
/// Ce que le tick écrit pour un candidat qui a franchi les sept termes,
/// selon ce qui lui est réellement arrivé.
pub struct Outcome { pub event: &'static str, pub message: &'static str }

pub fn outcome_for(disposition: Disposition) -> Outcome {
    match disposition {
        Disposition::Armed   => Outcome { event: REAPED_TOOL,        message: REAPED_MESSAGE },
        Disposition::Observe => Outcome { event: WOULD_DISPOSE_TOOL, message: WOULD_DISPOSE_MESSAGE },
    }
}
```

La boucle (`info!(event = outcome.event, …, outcome.message)`) et
`record_reaped` (`tool_name = outcome.event`) la consomment toutes deux. C'est
R4 : la divergence log/audit que le ticket a mesurée n'a pas de site où
renaître. `event` et `tool_name` restent **la même chaîne** — c'est
l'invariant actuel, on le conserve en le rendant explicite.

`tracing::info!` exige un message littéral ou une expression `%`/`?` : la forme
exacte (`message = outcome.message` en champ, ou `"{}", outcome.message`) est
laissée à l'implémenteur ; ce qui est fixé, c'est que **le texte vient de
`outcome_for`**, pas d'un second littéral.

### D3 — Le message observe nomme l'éligibilité et nie le retrait dans la même phrase

`WOULD_DISPOSE_MESSAGE = "worktree_reap: worktree de PR terminale éligible — observe, non retiré"`.
Un lecteur qui ne voit que le message (grep, tail, alerte) sait qu'il n'y a pas
eu d'action. `REAPED_MESSAGE` conserve le texte actuel, inchangé (R7).

### D4 — `reasoning` et `disposition` restent ; la redondance est voulue

Après D1, `disposition` est déductible du `tool_name`. On garde le champ et le
`disposition=` de `reasoning` (R5) : (i) la population « serait supprimé » se
lit d'une requête sur `worktree_reap_would_dispose` *ou* sur
`reasoning LIKE '%disposition=observe%'` — les deux existent déjà dans les
habitudes ; (ii) `mika2420_chaque_retrait_ecrit_une_ligne_daudit` asserte
`disposition=armed` et ne doit pas bouger ; (iii) un jour où une troisième
disposition apparaîtrait, le champ portera la nuance avant le nom.

### D5 — La ligne agrégée `worktree_reap_tick` ne change pas

Son compteur `disposed` compte les candidats *disposés* selon la disposition
courante, et la ligne porte `disposition=`. Le corps du ticket la juge
correcte (*« le tir lui-même est correct (disposed compté, rien supprimé) »*),
et la renommer en `would_dispose` en observe ferait diverger deux compteurs
pour une même population. Hors périmètre, nommé.

### D6 — Le garde SOLE WRITER s'étend au second nom

`mika2420_le_tool_name_daudit_a_un_seul_writer` scanne `src/` pour la chaîne
`worktree_reaped` hors du module. On lui ajoute `worktree_reap_would_dispose`
(écrite en deux morceaux, comme la première, pour que la garde ne se dénonce
pas elle-même). Même rationale : un second writer ne casserait rien et rendrait
la requête « population observée » inexacte en silence.

---

## Scope Boundaries

**Dans le périmètre :** le triplet (event, tool_name, message) par disposition ;
sa source unique ; le garde structurel ; les deux docs contractuels.

**Hors périmètre, nommé :**

- La ligne `worktree_reap_tick` et ses compteurs (D5).
- Le nom, la forme et la dédup des refus (`worktree_reap_skipped`).
- Les sept termes du prédicat, les réglages, le cap, le STOP à chaud.
- Renommer `Disposition::Armed`/`"armed"` en `enact` pour coller au vocabulaire
  du corps (M3) : ce serait un changement de format de fil pour un mot.
- Les plans historiques (`docs/plans/2026-09-20-002-fix-2420-…`) : on ne
  réécrit pas un plan livré ; c'est la doc vivante (module + CLAUDE.md) qui
  porte la vérité courante.
- Toute ligne d'audit **rétroactive** : les lignes `worktree_reaped` déjà
  écrites en observe avant ce fix restent telles quelles, identifiables par
  `reasoning LIKE '%disposition=observe%'`. Pas de migration de données.

---

## Implementation Units

### U1 — Le triplet et sa source unique (R1–R4, D1–D3)

`crates/mika-agent/src/worktree_reaper.rs` :

- Ajouter `pub const WOULD_DISPOSE_TOOL: &str = "worktree_reap_would_dispose";`
  à côté de `REAPED_TOOL`/`SKIPPED_TOOL`, avec un commentaire qui dit ce qu'il
  est : *« écrit en `observe` à la place de `REAPED_TOOL` — la population qui
  serait retirée »*.
- Ajouter `REAPED_MESSAGE` (texte actuel, inchangé) et `WOULD_DISPOSE_MESSAGE`
  (D3).
- Ajouter `Outcome` + `outcome_for(Disposition)` (D2).
- Dans `reap_terminal_worktrees`, remplacer `event = REAPED_TOOL` et le
  littéral par les champs de `outcome_for(cfg.disposition)`.
- Dans `record_reaped`, remplacer `REAPED_TOOL` par `outcome_for(disposition).event`.
  Le nom de la fonction peut devenir `record_outcome` (elle n'enregistre plus
  seulement des retraits) ; laissé au jugement de l'implémenteur, l'un ou
  l'autre est acceptable.
- Mettre à jour le commentaire de `REAPED_TOOL` (« SOLE WRITER … liste
  exacte des retraits ») pour qu'il devienne vrai dans les deux dispositions,
  et le tableau « Trois leviers » du doc de module (ligne `observe` : *« … écrit
  `worktree_reap_would_dispose`, jamais `worktree_reaped` »*).

### U2 — Le garde structurel (R6, D6)

`mika2420_le_tool_name_daudit_a_un_seul_writer` : itérer sur deux needles
(`"worktree"+"_reaped"`, `"worktree_reap_"+"would_dispose"`), même scan, même
allowlist vide, message d'échec nommant la needle fautive.

### U3 — Les tests (§ Verification Contract)

Dans `mod tests` de `worktree_reaper.rs`.

### U4 — Documentation (R8)

`CLAUDE.md` § reaper (mika#2420) :

- Levier `MIKA_WORKTREE_REAP_DISPOSITION` : *« writing its audit rows with
  `disposition: "observe"` »* → *« writing `worktree_reap_would_dispose` rows
  (never `worktree_reaped`) … »*.
- Surfaces opérateur : ajouter la requête
  `SELECT target_key, created_at FROM audit_events WHERE tool_name = 'worktree_reap_would_dispose' …`
  comme *« la population que le scan aurait retirée en observe »*, et ajouter
  `worktree_reap_would_dispose` à la liste des events greppables (mêmes champs
  que `worktree_reaped`, INFO, un par worktree éligible en observe).
- Sonde post-déploiement « start in `observe` for one tick, read the population
  that *would* be removed » : nommer la ligne à lire (`worktree_reap_would_dispose`).
- Une phrase de provenance : *« mika#2469 : avant ce fix, `observe` écrivait
  `worktree_reaped` avec `disposition=observe` ; les lignes antérieures au
  déploiement se distinguent par `reasoning` »*.

---

## Verification Contract

Commandes (depuis `mika/`) :

```
cargo test -p mika-agent worktree_reaper -- --nocapture
cargo clippy -p mika-agent --all-targets -- -D warnings
cargo fmt --check
```

Tests, tous dans `worktree_reaper::tests`, préfixe `mika2469_` :

- **T1 — `mika2469_en_observe_laudit_ne_revendique_pas_un_retrait`** (R2/R3 ;
  **rouge sur `main`**). `record_reaped(…, Disposition::Observe, …)` sur une DB
  mémoire ; asserte qu'il existe une ligne `tool_name == WOULD_DISPOSE_TOOL`
  portant `target_key = worktree:<WT>`, et **qu'aucune** ligne
  `tool_name == REAPED_TOOL` n'existe. Sur `main` la première assertion rougit
  (la ligne s'appelle `worktree_reaped`). Sortie rouge consignée dans le corps
  de la PR.
- **T2 — `mika2469_le_triplet_a_une_seule_source`** (R4, D2). `outcome_for(Armed)`
  rend `(REAPED_TOOL, REAPED_MESSAGE)` ; `outcome_for(Observe)` rend
  `(WOULD_DISPOSE_TOOL, WOULD_DISPOSE_MESSAGE)` ; les deux events diffèrent ;
  `WOULD_DISPOSE_MESSAGE` contient `"non retiré"` et **ne contient pas** le mot
  `"retiré"` seul en fin de phrase (précisément : ne se termine pas par
  `" retiré"`) ; `REAPED_MESSAGE` est le texte historique à l'octet près.
- **T3 — `mika2420_chaque_retrait_ecrit_une_ligne_daudit`** (existant, R7) :
  reste vert, non modifié — c'est le contrôle que le chemin `armed` n'a pas
  bougé.
- **T4 — `mika2420_le_tool_name_daudit_a_un_seul_writer`** (U2, R6) : vert à
  HEAD pour les deux needles, allowlist vide.
- **T5 — contrôle négatif de T1** : mutation `Observe → Armed` dans
  `outcome_for` ; T1 rougit **seul** (T2 rougit aussi — attendu, il pine la
  même fonction ; T3 reste vert). Rapporté dans la PR, puis mutation retirée.
  Commiter avant de muter (`feedback_commit_before_mutate_restore_negative_control`).

Le texte du log tracing n'est pas capturé en test (pas de subscriber de test
dans ce crate hors `memory/`) ; c'est D2 qui le couvre : le message que
`info!` émet **est** `outcome.message`, et T2 pine ce texte.

---

## Definition of Done

- [ ] U1–U4 livrés.
- [ ] T1, T2, T4 verts ; T3 vert **non modifié** ; sortie **rouge** de T1 sur
      `main` consignée dans le corps de la PR ; contrôle négatif T5 rapporté.
- [ ] `grep -n 'worktree_reaped' crates/mika-agent/src` ne rend que
      `worktree_reaper.rs` (garde T4).
- [ ] `grep -n '"worktree_reap: worktree de PR terminale retiré"' crates/` rend
      **un seul** site (la constante `REAPED_MESSAGE`).
- [ ] Aucune valeur de réglage, aucun terme T1–T7, aucun compteur du tick,
      aucune migration modifiés ; `schema_version` inchangée.
- [ ] `CLAUDE.md` § reaper et le doc de module disent la même chose que le code
      (R8) ; la requête opérateur « retraits » est exacte dans les deux
      dispositions.
- [ ] Sonde post-merge (une seule, bornée) : un tick en
      `MIKA_WORKTREE_REAP_DISPOSITION=observe` sur la station, s'il existe au
      moins un candidat, produit `event=worktree_reap_would_dispose` et une ligne
      d'audit du même `tool_name`, et **zéro** `worktree_reaped` ; le worktree est
      toujours sur disque. S'il n'y a aucun candidat au moment de la sonde, le
      dire (« population vide, sonde non concluante ») plutôt que de conclure —
      classe mika#2249 (*zéro = absence de mesure*).

---

## Acceptance criteria

Transcrits du corps de mika#2469 (§ Attendu), et de sa contrainte de contexte :

- **AC1** — En mode observe, le log reflète l'intention, pas l'action : le
  message dit que le worktree est éligible et **non retiré**.
- **AC2** — Le mot « retiré » et l'event `worktree_reaped` sont réservés au mode
  où la suppression a réellement lieu (`armed`, M3).
- **AC3** — Le tir lui-même reste correct : disposed compté, rien supprimé en
  observe ; le chemin `armed` inchangé.

**Correspondance :** AC1 ← U1/D3, T2 ; AC2 ← U1/D1/D2, T1 (rouge sur main),
T4 ; AC3 ← R7, T3 (non modifié), D5. L'extension à la surface d'audit (R2/R3)
n'est pas une AC du corps : c'est M1 — la même constante nourrit les deux
surfaces, et AC2 dit « l'event `worktree_reaped` », qui est cette constante.
Corriger le log seul aurait laissé la requête documentée en CLAUDE.md mentir de
la même façon ; c'est inscrit ici pour que l'architecte le voie comme un choix,
pas comme une dérive.

---

## Fire-Disposition

**(a) exception nommée en allowlist — livrée VIDE.**

Ce plan ne livre qu'un détecteur nouveau : la seconde needle du garde SOLE
WRITER (U2/T4). Elle ne peut pas tirer sur le code existant, et c'est
vérifiable avant d'écrire une ligne : la chaîne `worktree_reap_would_dispose`
n'existe nulle part dans `crates/` à HEAD (`grep -rn would_dispose crates/`
rend vide au moment du groom). L'allowlist est livrée vide et son vide est
asserté ; si un jour la garde tire, la résolution est de retirer le second
site, jamais d'y ajouter une entrée (forme mika#2323, mika#1940, mika#2267).

Aucun détecteur n'est livré désarmé.

---

## Suivi (hors périmètre, nommé)

- Aucun ticket à ouvrir. La ligne `worktree_reap_tick` (D5) est nommée hors
  périmètre sans besoin mesuré ; si un lecteur est un jour trompé par
  `disposed` en observe, ce sera n=1 et le moment de le nommer.
