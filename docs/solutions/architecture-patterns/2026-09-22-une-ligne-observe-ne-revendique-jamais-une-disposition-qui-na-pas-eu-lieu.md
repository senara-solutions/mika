---
title: "Une ligne en mode observe ne revendique jamais une disposition qui n'a pas eu lieu"
date: 2026-09-22
category: architecture-patterns
module: mika-agent/worktree_reaper, mika-agent/task_engine
problem_type: architecture_pattern
component: mika-agent
severity: medium
applies_when:
  - "Un scan porte une disposition gardée (armed / observe, enact / dry-run) et écrit un event tracing, un tool_name d'audit ou un message par candidat"
  - "Le nom d'un event ou d'un tool_name est celui d'une ACTION (reaped, killed, expired) et sert de requête opérateur « liste exacte de ce que la boucle a fait »"
  - "On ajoute un mode d'observation à un mécanisme qui n'en avait pas, ou on relit un mécanisme livré avec ce patron"
tags: [observe-mode, dry-run, audit-events, tool-name, sole-writer, disposition, mika-2249, mika-2469, wire-format]
---

# Une ligne en mode observe ne revendique jamais une disposition qui n'a pas eu lieu

## Context

Deux scans du moteur ont livré un mode d'observation (« la détection est
inconditionnelle, seule la disposition est gardée », patron mika#2249) et,
deux fois, la ligne écrite en observe a dit qu'une action avait eu lieu :

- **mika#2249 → mika#2272** (faucheur de pilotes silencieux) : l'event et le
  `tool_name` sont `pilot_silent_stall` — le nom de la *détection* — et ce qui
  s'est réellement passé est porté par `disposition_armed`, `transitioned`, et
  un `after_value` qui « dit ce qui s'est réellement passé, pas ce qui était
  voulu ». Forme (a) : un nom neutre, des champs qui disent l'effet.
- **mika#2469** (faucheur de worktrees, mika#2420) : l'event et le `tool_name`
  étaient une **seule constante** `REAPED_TOOL = "worktree_reaped"`, au nom
  d'une *action*, servie sur les deux surfaces (`info!` et `audit_events`) ; en
  `observe` la ligne disait « worktree de PR terminale **retiré** » pendant que
  le worktree était toujours sur disque, et la requête opérateur documentée
  `SELECT … WHERE tool_name = 'worktree_reaped'` — « la liste exacte des
  retraits » — comptait des observations parmi les retraits. Tir mesuré le
  2026-09-21 16:10:04Z sur `fix-2184-…`.

Aucune entrée du corpus ne nommait le patron général ; le learnings-researcher
de la revue de mika#2469 l'a relevé comme récurrence n=2 sur deux sous-systèmes.

## Guidance

**La règle** : une ligne (log ou audit) ne revendique jamais une disposition qui
n'a pas eu lieu. Le mode observe écrit ce que le scan *ferait*, jamais ce qu'il
*a fait*.

**Deux formes valides, et c'est le nom existant qui tranche laquelle :**

| le nom existant est… | forme | exemple |
|---|---|---|
| celui d'une **détection** (`pilot_silent_stall`) | (a) un event neutre + des champs qui disent l'effet (`disposition_armed`, `transitioned`, `after_value` réel) | mika#2272 |
| celui d'une **action** (`worktree_reaped`), déjà contractuel (requête opérateur, HALT, garde SOLE WRITER) | (b) **réserver** ce nom à l'action effective et donner un nom propre au cas observe (`worktree_reap_would_dispose`), dans la même famille de préfixe | mika#2469 |

Ne pas neutraliser un nom d'action contractuel (forme (a) appliquée à (b)) : il
faudrait réécrire chaque requête historique pour y ajouter `removed = true`, et
une requête non mise à jour se mettrait à compter des observations en silence.

**Une seule source pour le triplet.** Le point qui a produit le défaut de
mika#2469 est qu'`event` (tracing) et `tool_name` (audit) étaient la même
constante lue à deux sites, et que le message était un troisième littéral. La
correction est une fonction pure par disposition, consommée par les deux
surfaces :

```rust
pub struct Outcome { pub event: &'static str, pub message: &'static str }

pub fn outcome_for(disposition: Disposition) -> Outcome {
    match disposition {
        Disposition::Armed   => Outcome { event: REAPED_TOOL,        message: REAPED_MESSAGE },
        Disposition::Observe => Outcome { event: WOULD_DISPOSE_TOOL, message: WOULD_DISPOSE_MESSAGE },
    }
}
// info!(event = outcome.event, …, "{}", outcome.message);
// db.log_audit_event(session_id, outcome.event, …)
```

Toute ligne dérivée du même site — y compris le **WARN d'échec d'écriture** —
lit le même `outcome.event` ; sinon la ligne d'erreur ré-introduit le
mensonge (`audit write failed (reaped)` en observe, relevé par la revue).

**Le message observe nie l'action dans la même phrase** : « … éligible —
observe, non retiré ». Un lecteur qui ne voit que le message (grep, tail,
alerte) sait qu'il ne s'est rien passé ; le champ `disposition` en fin de ligne
ne suffit pas, c'est précisément ce que le lecteur de mika#2469 n'a pas vu.

**Le garde SOLE WRITER couvre les deux noms**, à allowlist vide, needles
écrites en deux morceaux pour ne pas se dénoncer, exclusion de test par le
lecteur partagé (`source_scan::is_test_source_path`, voir
`2026-09-19-audit-scanners-sources-structurels.md`). Un second writer de l'un
ou l'autre nom rendrait une requête opérateur inexacte sans rien casser.

**Les champs restent.** `disposition` sur les deux lignes et `disposition=<x>`
dans `reasoning` sont conservés même s'ils deviennent déductibles du nom : les
tests et requêtes existants les lisent, et une troisième disposition future
portera la nuance avant le nom.

## Why This Matters

Le log est la surface d'incident (classe mika#2041) et la requête `WHERE
tool_name = '<action>'` est le garde-fou opérateur d'un scan destructif. Une
ligne d'observation qui porte le nom de l'action falsifie les deux à la fois,
sans qu'aucun test comportemental ne rougisse : rien n'est faux dans la
*décision*, seul le *récit* l'est. C'est pourquoi la correction est
structurelle (une source, deux noms, un garde) et non un `if` dans le message.

## When to Apply

- À la conception d'un mode observe / dry-run : décider la forme (a) ou (b)
  **avant** d'écrire la première ligne, à partir de ce que le nom existant
  désigne.
- En relecture d'un scan qui écrit `audit_events` : chercher l'event et le
  `tool_name` écrits sur le chemin observe et vérifier qu'ils ne nomment pas
  l'action ; chercher aussi les lignes dérivées (WARN d'échec, tick agrégé).
- Quand une requête opérateur est documentée comme « liste exacte de ce que
  la boucle a fait » : le nom qu'elle filtre doit être réservé à l'action.

## Examples

Avant (mika#2420, en `observe`) :

```
{"message":"worktree_reap: worktree de PR terminale retiré","event":"worktree_reaped","disposition":"observe"}
```

Après (mika#2469, même tick) :

```
{"message":"worktree_reap: worktree de PR terminale éligible — observe, non retiré","event":"worktree_reap_would_dispose","disposition":"observe"}
```

et la requête « retraits » redevient exacte pour toute ligne écrite après le
déploiement ; les lignes antérieures se distinguent par
`reasoning LIKE '%disposition=observe%'`, pas par le nom (pas de migration).

**Deux pièges trouvés en chemin, à connaître :**

- *« Rouge sur `main` » est parfois inatteignable.* Quand le test de
  non-régression référence une constante nouvelle, il ne compile pas sur
  `main` ; la preuve rouge se produit **avant câblage** — commiter la
  constante et le test (neutres), tirer le rouge, câbler. Nommer cet ordre
  dans le plan plutôt que d'écrire « rouge sur main ».
- *Un cap d'écritures s'applique aussi en observe.* Dans `worktree_reaper`,
  `budget -= 1` précède le `match cfg.disposition`, donc un tick en observe
  nomme au plus `MIKA_WORKTREE_REAP_MAX_PER_TICK` (3) candidats : « un tick en
  observe puis armer » n'est pas un dry-run de la population. La sonde doit
  durer `ceil(N / cap)` ticks ou relever le cap pour la passe d'observation
  (revue mika#2469, appliqué dans `CLAUDE.md` § reaper).

## Related

- mika#2469 (ce fix), mika#2420 (le scan), mika#2249 / mika#2272 (le précédent
  et sa forme (a)), mika#2041 (le log comme surface d'incident)
- `docs/solutions/architecture-patterns/2026-09-19-audit-scanners-sources-structurels.md`
  — la frontière test/production du garde SOLE WRITER
- `docs/solutions/cross-repo-patterns/garde-suppression-worktree-allowlist-2026-09-20.md`
  — même module, l'autre invariant (fail-safe vers « garder »)
- `crates/mika-agent/src/worktree_reaper.rs` : `outcome_for`,
  `mika2420_le_tool_name_daudit_a_un_seul_writer`, `mika2469_*`
