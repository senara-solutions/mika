# mika#1943 — Un chemin qu'on ne peut pas prouver worktree n'est pas supprimé

> - **Ticket:** senara-solutions/mika#1943
> - **Type:** chore (garde structurelle + rectification d'un AC)
> - **Priorité:** p2-normal (filet de sûreté, pas de fuite active mesurée)

## Goal Capsule

Le ticket porte deux AC. **Ils ne se ferment pas au même endroit, et l'un des
deux n'est pas exécutable depuis une session dispatchée.** La première moitié de
ce plan est la rectification de cette asymétrie, mesurée plutôt que supposée ;
la seconde est le remède de l'AC2, dont l'objet réel n'est pas celui que le
ticket nomme.

---

## M0 — Ce que la mesure déplace dans le ticket (premier livrable)

### M0.1 — L'AC1 n'est pas exécutable depuis un pilote, et c'est structurel

L'AC1 demande d'exhumer le transcript du nettoyage du 28/07 et d'en tirer un
inventaire. Les trois sources qu'il nomme vivent **hors du sandbox** dans lequel
tourne toute session dispatchée :

| source nommée par l'AC1 | état dans le sandbox | ce que ça prouve |
|---|---|---|
| `~/.claude/projects/-data-workspace-mika-platform/` | **absent** | rien — `/home` est un `tmpfs` |
| `/var/spool/claude-mail/samidarko/archive/` | **inexistant** | rien — non monté |
| mémoire MPC | hors périmètre | — |

La mesure qui tranche, lue dans `/proc/self/mounts` :

```
tmpfs /home tmpfs rw,nosuid,nodev,relatime,mode=755,uid=1000,gid=1000 0 0
```

`/home` est un **tmpfs créé pour ce sandbox**. Seuls des chemins précis y sont
bind-montés en lecture seule (`.claude/plugins`, `.claude/settings.json`,
`.claude/commands`, `.claude/hooks`, `.cargo/*`, `.rustup`, …).
**`~/.claude/projects/` n'en fait pas partie.** Corollaire vérifié : l'unique
`*.jsonl` visible sur toute la machine est celui de la session courante, créé
quelques minutes plus tôt, et `~/.claude/projects/-data-workspace-mika-platform-mika/`
porte un mtime du jour avec un seul sous-répertoire `memory/` vide.

**Donc : « le fichier n'existe pas » signifie ici « le fichier n'est pas monté
dans mon sandbox », jamais « le fichier n'existe pas sur l'hôte ».** C'est la
doctrine déjà écrite pour le reaper mika#2277 — *un signal qui ne peut pas être
lu n'est jamais un terme satisfait* — appliquée à une session au lieu d'un
prédicat.

**Ce que ça interdit, et c'est le point.** Écrire
`docs/solutions/incident-2026-07-28-bbytaa-cleanup-inventory.md` en concluant
« voici les chemins supprimés hors `target/` » depuis cette vue serait
**fabriquer une mesure** : un document d'incident qui affirme un inventaire
qu'aucune source consultable ne soutient. C'est la classe #953, et un inventaire
faux portant l'autorité d'un doc d'incident est strictement pire que pas
d'inventaire — il clôt la question en se trompant.

**Second obstacle, indépendant du sandbox.** `~/.claude/.last-cleanup` porte la
date du jour et `settings.json` ne configure **aucune** rétention, donc le défaut
Claude Code (30 jours) s'applique. Le 28/07 est à **54 jours**. Même exécuté sur
l'hôte, l'AC1 a une probabilité faible d'aboutir — mais **cette phrase-là ne peut
pas être vérifiée d'ici non plus**, et le plan ne l'affirme pas.

**Disposition : l'AC1 sort du périmètre de cette PR et devient un geste
opérateur**, avec sa procédure écrite (U4). Il n'est ni abandonné ni déclaré
satisfait. Si l'opérateur l'exécute et trouve les sources, le doc s'écrit avec
une mesure derrière lui ; s'il ne les trouve pas, le résultat — « les sources ont
été purgées par la rétention » — est lui-même un fait à consigner, et un fait
honnête.

### M0.2 — L'AC2 a un objet réel, mais l'asymétrie est inversée

Le ticket suppose que les nettoyages automatisés n'ont pas de garde. **Le plus
récent en a une, et elle est plus forte que celle qu'il demande.**
`crates/mika-agent/src/worktree_reaper.rs` (mika#2420, mergé hier, `fc96a341`)
est un scan récurrent qui fait `git worktree remove --force` toutes les dix
minutes. Son terme T1 :

```rust
pub const MANAGED_WORKTREE_SEGMENT: &str = "/.claude/worktrees/";

pub fn is_managed_worktree_path(path: &str) -> bool {
    let p = Path::new(path);
    if !p.is_absolute() { return false; }
    if p.components().any(|c| matches!(c, Component::ParentDir)) { return false; }
    path.contains(MANAGED_WORKTREE_SEGMENT)
}
```

plus une **re-vérification après canonicalisation** juste avant la disposition
(`canonical_path_is_managed`), qui élimine en outre le lien symbolique.

C'est une **allowlist positive** : le chemin doit *prouver* qu'il est un worktree
géré. La denylist `^/data/workspace/[^/]+/?$` que l'AC2 prescrit est strictement
**plus faible** — elle protège ce qu'elle énumère, et `/data/workspace/bbytaa`
n'était protégé par aucune liste, il était protégé par btrbk.

**Le remède retient donc l'allowlist, pas la denylist**, pour trois raisons dont
la troisième est décisive :

1. Une denylist est fausse le jour où un répertoire précieux n'y figure pas —
   c'est-à-dire le jour où elle servirait.
2. Deux sémantiques opposées pour une même question dans un même dépôt est la
   divergence programmée que `grooming_marker` (mika#2158) a dû fermer une fois :
   deux prédicats répondant différemment à « ce chemin est-il supprimable ».
3. **`/data/workspace/` est le disque de Vincent, pas une propriété du système.**
   Un garde-fou qui code en dur ce préfixe ne protège que gentux et devient muet
   sur toute autre machine — un garde-fou qui *paraît* poser une règle générale.
   `/.claude/worktrees/` est, lui, une propriété **structurelle** du layout.

### M0.3 — Où le trou est réellement, et il est plus étroit que le ticket

`skills/bundled/_shared/dispatch-lib.sh` — l'ancien consommateur, qui tourne à
**chaque** dispatch — porte cinq sites destructifs dont les chemins sont dérivés
sans validation. La racine est ligne 2233 :

```bash
WORKTREE_DIR=$("$PLATFORM_DIR/scripts/derive-worktree-path" --branch "$BRANCH" --repo "$REPO")
```

**Le code de sortie n'est pas vérifié**, et le fichier n'a ni `set -e` ni
`set -u` (`#!/bin/bash` nu, ligne 1). Si le script échoue ou est absent,
`WORKTREE_DIR` devient la chaîne vide et se propage en silence :

| ligne | site | effet si `WORKTREE_DIR` / `$wt` est vide |
|---|---|---|
| 2245 | `[ "$existing_wt" != "$WORKTREE_DIR" ]` | **tout** worktree existant devient « non-canonique » |
| 2262 | `git worktree remove --force "$existing_wt"` | suppression décidée sur une comparaison faussée |
| 2269 | `git worktree remove --force "$WORKTREE_DIR"` | `remove --force ""` |
| 1610 | `rm -rf "$wt/.iterate"` | `rm -rf "/.iterate"` |
| 5273 | `rm -rf "$findings_dir"` | gardé par `[ -d ]`, chemin non validé |

**Sévérité réelle, dite honnêtement plutôt que gonflée.** `git worktree remove`
n'est pas `rm -rf` : git refuse un chemin qui n'est pas un worktree enregistré,
donc 2262/2269 sont déjà protégés — **par git, pas par dispatch-lib**. Et
`rm -rf "/.iterate"` est inoffensif en pratique. **Ce ticket ne répare donc pas
une fuite active mesurée ; il pose une garde structurelle sur une classe dont
l'incident du 28/07 a montré le coût.** C'est exactement ce que sa priorité p2
annonce, et le plan ne prétend pas davantage. Ce qui *est* réparé et n'était
protégé par rien, c'est **2245** : une comparaison d'égalité dont un côté peut
être vide et qui élit une cible de suppression.

---

## Product Contract

**Invariant.** Aucun site de suppression automatisée de `dispatch-lib.sh`
n'opère sur un chemin qu'il ne peut pas prouver être un worktree géré. Un chemin
invalide, vide, relatif, porteur de `..`, ou hors de `.claude/worktrees/` est
**refusé et dit**, jamais supprimé.

**Asymétrie qui décide de tout, et elle est écrite avant le reste.** Un faux
négatif laisse un worktree résiduel sur le disque : le reaper mika#2420 le
ramassera au prochain tick, ou l'opérateur. Coût borné, quelques Go, temporaire.
Un faux positif supprime un répertoire qui n'est pas un worktree : irréversible,
et c'est l'incident du 28/07. **Donc tout terme illisible conserve.** Même sens
que le reaper — délibérément, pour que les deux gardes ne puissent pas se
contredire.

**Hors périmètre, nommé.** `make prune-worktrees`, `worktrees-audit` et
`worktrees-clean` (couches A/B de #1694) vivent dans le dépôt **mika-platform**,
absent de ce workspace. Ils restent le geste manuel. Ce plan ne peut pas les
modifier et ne le prétend pas ; U5 ouvre le ticket de suivi.

---

## Implementation Units

### U1 — `_assert_removable_worktree_path`, lecteur unique

`skills/bundled/_shared/dispatch-lib.sh`. Une fonction, **seul site** qui décide
si un chemin est supprimable. Retourne non-zéro et écrit sur `stderr` sinon.

Quatre termes conjonctifs, alignés terme pour terme sur `is_managed_worktree_path` :

1. non vide ;
2. absolu (commence par `/`) ;
3. aucun composant `..` ;
4. contient `/.claude/worktrees/`.

Le refus émet `dispatch_lib_unsafe_removal_refused` sur `stderr` avec le chemin
et le terme qui a échoué — sans quoi un refus se lirait exactement comme une
absence de travail (classe mika#2205).

**Pourquoi une fonction et pas une garde à chaque site :** cinq sites, donc cinq
occasions de diverger. C'est la leçon que `grooming_marker` a dû engraver une
fois dans ce dépôt.

### U2 — Câbler les cinq sites, et fermer la racine

- **Racine (l. 2233)** : vérifier le code de sortie de `derive-worktree-path`
  **et** la non-vacuité du résultat. Échec ⇒ abandon du dispatch avec un message
  nommant le script, jamais une chaîne vide propagée. C'est le correctif qui
  rend les quatre autres redondants dans le cas nominal — et on pose quand même
  les quatre, parce qu'une garde qui dépend d'un seul point de contrôle en amont
  n'est pas une garde.
- **l. 2245** : ne comparer que si `WORKTREE_DIR` est non vide ; sinon ne rien
  élire.
- **l. 2262 / 2269 / 1610 / 5273** : appel de `_assert_removable_worktree_path`
  avant la suppression, `|| return 1` (ou `|| true` selon la criticité du site,
  décidée site par site à l'implémentation — jamais une suppression sur refus).

### U3 — Test injection-verified (l'AC2 le demande explicitement)

Dans `skills/bundled/_shared/test-dispatch-lib.sh`, déjà câblé en CI
(`make test-dispatch-lib`, mika#1772).

**Contrôles positifs** — refusés : `""`, `/data/workspace/foo`,
`/data/workspace/bbytaa`, `relatif/x`, `/data/workspace/mika-platform/.claude/worktrees/../../../etc`.
**Contrôle négatif** — accepté :
`/data/workspace/mika-platform/.claude/worktrees/chore-1943-.../mika`.

**Le contrôle négatif est ce qui donne un sens au positif** (doctrine mika#2420) :
sans lui, une fonction qui refuse *tout* passerait la suite en vert tout en
cassant chaque dispatch.

**Vérification par injection, exigée par l'AC2 et faite à la main au moment de
livrer :** neutraliser chaque terme de la conjonction *un par un* et observer le
test rougir à chaque fois. Neutraliser les quatre d'un coup ne prouve rien — une
conjonction ne se teste pas en désarmant tous ses termes ensemble (leçon
mika#2277).

### U4 — Procédure opérateur pour l'AC1

`docs/operator/` : la procédure exécutable **sur l'hôte, hors sandbox** —
chemins à consulter, fenêtre de rétention à vérifier d'abord, et la consigne
explicite que l'absence de source se consigne comme telle plutôt que de produire
un inventaire reconstitué. Le doc `incident-2026-07-28-…` reste à écrire *après*
cette exécution, et seulement si elle rend quelque chose.

### U5 — Suivi

Un ticket sur **mika-platform** pour porter la même garde à `make prune-worktrees` /
`worktrees-audit` / `worktrees-clean`, en réutilisant la sémantique allowlist
fixée ici — pas une denylist parallèle.

---

## Verification Contract

- `make test-dispatch-lib` vert, contrôle négatif inclus.
- Injection terme par terme : quatre neutralisations, quatre rougissements.
- Un dispatch réel aboutit (le contrôle négatif en conditions réelles).
- `grep dispatch_lib_unsafe_removal_refused` sur les logs de dispatch :
  **régime attendu zéro ligne**. Une occurrence est un chemin que la dérivation
  a produit et que la garde a arrêté — c'est-à-dire la racine rendue visible, et
  elle alimente U2 plutôt qu'un élargissement de la garde.

---

## Definition of Done

- `_assert_removable_worktree_path` existe, est le seul décideur, et les cinq
  sites l'appellent.
- Le code de sortie de `derive-worktree-path` est vérifié ; aucune chaîne vide ne
  se propage vers un site destructif.
- Le test injection-verified est dans `test-dispatch-lib.sh` et passe en CI.
- La procédure opérateur AC1 est écrite ; le doc d'inventaire n'est **pas**
  fabriqué.
- Le ticket de suivi mika-platform est ouvert.

---

## Acceptance criteria

Transcrits du corps du ticket, avec leur disposition mesurée.

**AC1 — Exhumer le transcript du 28/07 + inventaire.**
*Disposition : hors périmètre de cette PR, converti en geste opérateur (U4).*
Non exécutable depuis une session dispatchée — `/home` est un tmpfs et
`~/.claude/projects/` n'y est pas monté (M0.1). Produire le doc d'inventaire
depuis cette vue serait fabriquer une mesure. Le doc
`docs/solutions/incident-2026-07-28-bbytaa-cleanup-inventory.md` **n'est pas
écrit par cette PR**.

**AC2 — Garde-fou allowlist.** *Disposition : livré, avec une sémantique
renforcée et un périmètre rectifié.*

- **Deny-list `/data/workspace/*` jamais auto-supprimable** — satisfait *a
  fortiori* par une **allowlist positive** (le chemin doit contenir
  `/.claude/worktrees/`), strictement plus forte. Justification en M0.2, dont la
  raison décisive : `/data/workspace/` est une propriété de la machine, pas du
  système.
- **Gate structurelle** — livrée sous la forme des quatre termes de U1, qui
  couvrent le cas `^/data/workspace/[^/]+/?$` et, en plus, le vide, le relatif
  et la traversée par `..`.
- **Test injection-verified** — U3, avec neutralisation terme par terme.
- **Périmètre** : `dispatch-lib.sh` dans cette PR. `worktree_reaper.rs` est
  **déjà conforme** et n'est pas touché. `make prune-worktrees` et les couches
  A/B de #1694 sont dans mika-platform, hors de ce dépôt → U5.

---

## Sources

- `crates/mika-agent/src/worktree_reaper.rs:128,471,494` — allowlist T1 et
  canonicalisation (mika#2420).
- `skills/bundled/_shared/dispatch-lib.sh:1610,2233,2245,2262,2269,5273` — les
  cinq sites et leur racine.
- `/proc/self/mounts` — `tmpfs /home`, mesure qui tranche l'AC1.
- `~/.claude/.last-cleanup` (2026-09-20) + `settings.json` sans rétention
  configurée — second obstacle à l'AC1.
- mika#2158 (lecteur unique), mika#2205 (un refus muet se lit comme une absence
  de travail), mika#2277 (un signal illisible n'est jamais un terme satisfait),
  mika#2420 (le contrôle négatif donne son sens au positif).
