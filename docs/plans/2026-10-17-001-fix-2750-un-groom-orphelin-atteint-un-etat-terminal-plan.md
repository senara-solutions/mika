# mika#2750 — Un parent de groom dont le callback a délivré atteint un état terminal nommé

> Plan content-only produit par `/mika-groom-plan-only`. Ticket : senara-solutions/mika#2750.
> Labels portés : `bug`, `p2-normal`, `ready`, `dispatch:loop`. Aucun `loop-substrate`.

---

## 1. Contexte : le trou est déjà nommé dans le code, et il porte un numéro

Le ticket décrit une tâche ready-label `fef8cd36` restée `in_progress` six heures,
sans processus, sans re-drive, comptée en vol par l'auto-feeder à chaque tick.

La cause n'est pas à chercher : elle est écrite en toutes lettres dans le
doc-comment de la requête qui aurait dû la ramasser —
`crates/mika-agent/src/db/tasks.rs:1984-2000`, au-dessus de
`find_orphaned_parent_tasks` :

```
/// Groom-class callbacks (mika#1001) are NOT reaped here — their expected
/// artifact is a plan commit pushed to the branch, not a PR url. Groom-class
/// leak detection is a separate follow-up (mika#1118 Option B).
```

et le prédicat qui l'applique, `crates/mika-agent/src/db/tasks.rs:2022` :

```sql
AND COALESCE(child.dispatch_class, 'implement') = 'implement'
```

mika#2750 **est** ce follow-up. Il ne découvre pas un défaut : il vient payer une
dette que le code annonce depuis mika#1001.

### 1.1 Les trois filets existants, et pourquoi aucun des trois ne tient cette row

Le balayage périodique du moteur (`DB_SCAN_INTERVAL_TICKS`, toutes les 60 s —
`crates/mika-agent/src/task_engine/engine.rs:1284-1345`) porte déjà cinq
réparateurs. Trois pouvaient plausiblement couvrir cette population. Aucun ne le
fait, et chacun manque pour une raison **différente** :

| filet | site | pourquoi il ne voit pas `fef8cd36` |
|---|---|---|
| callback watchdog (#959) | `check_callback_process_liveness` | son premier prédicat est `process_id IS NOT NULL`, et il traite un process **mort sans callback**. Ici le callback **a délivré** : il a fait son travail, il est hors sujet |
| reaper de parents orphelins (#871/#2279) | `reap_orphaned_parent_tasks` → `find_orphaned_parent_tasks` | la clause `dispatch_class = 'implement'` citée ci-dessus. L'enfant est de classe `groom` |
| fermeur de parents de dispatch (mika#2405) | `settle_dispatch_parents` → `find_settleable_dispatch_parents` | `COALESCE(parent.source,'') != 'self_dev'` (`db/tasks.rs:2540`). Notre parent **est** `self_dev` |

Le balayage des phantoms (mika#1712, `find_phantom_tracking_tasks`) sélectionne
bien `action_type='none'` + `status='in_progress'` — ce que la row porte — mais
exige `process_id IS NULL` et passe ensuite chaque candidat par une garde de
liveness. Le ticket relève un `process_id` **vide**, pas nécessairement NULL, et
la row a un enfant callback : la caractériser par ce filet-là reviendrait à la
traiter comme une row fantôme, ce qu'elle n'est pas — elle a reçu un dispatch
réel, qui a rendu un résultat réel. Le bon filet est celui qui raisonne sur le
couple parent/callback, c'est-à-dire le deuxième.

### 1.2 Le défaut est donc **un prédicat**, pas un contrat de prompt manquant

Le ticket observe, à juste titre, que le § 3g de
`skills/bundled/self-dev-webhook-ready-label/system_prompt.md` prévoit un retry
et que le tour de callback ne l'a pas fait. Mais la leçon structurelle est
ailleurs : **le dispatch ready-label n'est plus tenu par le prompt depuis
mika#1384.** `crates/mika-agent/src/server/ready_label_handler.rs` pré-crée la
tâche, résout la classe, et **spawne lui-même le subprocess** (`ReadyLabelGate::Dispatched`,
événement `ready_label_engine_dispatched` à la ligne 1631 — exactement celui que
le ticket relève à 16:02:25Z).

Le dispatch est structurel ; la **sortie** du dispatch ne l'est pas. C'est
l'asymétrie que ce vol corrige, et il la corrige du côté le moins cher : en
rendant le réparateur existant conscient de la seconde classe, plutôt qu'en
réécrivant la reprise de main dans le moteur.

---

## 2. La décision : l'état terminal, et le re-drive qu'il produit gratuitement

L'« Attendu » du ticket offre deux issues et n'en exige qu'une :

1. le re-drive — le retry unique du § 3g part réellement ;
2. l'état terminal nommé — la tâche quitte `in_progress`, l'auto-feeder cesse de
   la compter, un événement nommé dit pourquoi.

**Ce plan livre (2), et (1) en découle sans une ligne de code de re-dispatch.**
La chaîne est vérifiable, maillon par maillon :

- La transition vers `failed` sort la row de l'index
  `idx_tasks_manual_active_ref_url`, dont la définition
  (`crates/mika-agent/src/db/migrations.rs:743-747`) exclut explicitement
  `status IN ('completed','cancelled','failed','delivered')`.
- L'auto-feeder cesse donc de compter l'issue en vol : c'est la même mécanique
  que mika#2045 documente pour les parents `pending`
  (`engine.rs:1777-1780` : « *That drops it out of
  `idx_tasks_manual_active_ref_url` … and the next `ready` sweep creates a fresh
  task for the issue* »).
- Le ticket **garde son label `ready`** : contrairement au § 1 du prompt
  ready-label, le chemin moteur ne fait aucun `gh issue edit --remove-label`
  (zéro occurrence de `--remove-label` dans
  `crates/mika-agent/src/server/ready_label_handler.rs` ; le seul site du
  répertoire `server/` est `draft_pr_opened_handler.rs:159`). Le ticket reste
  donc éligible au prochain balayage.
- Le prochain balayage ready crée une tâche fraîche et redispatche le groom :
  c'est le re-drive, produit par le relâchement du verrou plutôt que par un
  re-spawn.

### 2.1 Pourquoi ne PAS implémenter le re-drive explicite

Un re-spawn depuis le moteur demanderait de reconstruire un `action_config` de
dispatch, de re-résoudre le handler, et de re-spawner un subprocess — le chemin
que `rearm_deferred_callback` tient pour les wrappers `pending`, et qui pèse à
lui seul plus que la borne par vol de ce ticket. Il introduirait surtout un
**second producteur de dispatch groom** à côté du ready-label handler, là où
mika#2506 maintient un recensement fermé des sites de dispatch pilote
(`canonical_tokens.rs:3391-3488`) précisément pour que ce nombre n'augmente pas
en silence.

Relâcher le verrou coûte un prédicat ; re-spawner coûte un producteur. À issue
équivalente au sens de l'« Attendu », le prédicat gagne.

---

## 3. Les livrables, site par site

### D1 — `find_orphaned_parent_tasks` devient paramétrée par la classe

**Fichier :** `crates/mika-agent/src/db/tasks.rs` (~l. 1984-2049).

Signature :

```rust
pub fn find_orphaned_parent_tasks(
    &self,
    agent_id: &str,
    grace_seconds: i64,
    dispatch_class: &str,
) -> Result<Vec<OrphanedParentTask>>
```

Seul changement au SQL — la constante devient un paramètre lié :

```sql
AND COALESCE(child.dispatch_class, 'implement') = ?3
```

Trois points que le doc-comment doit porter, parce qu'ils ne se lisent pas sur
le SQL :

- **Le `COALESCE(..., 'implement')` reste, et c'est une propriété, pas un
  résidu.** Une row antérieure à la v34 porte `dispatch_class` NULL et vaut
  `implement` ; un appel en `'groom'` ne peut donc **jamais** la ramasser. La
  rétro-compatibilité du schéma est préservée du seul côté où elle a un sens.
- **Le prédicat `$.claude_pilot.pr_url IS NULL` est conservé pour les deux
  classes.** Pour `groom` il n'est pas le critère d'artefact (le ticket le dit :
  l'artefact d'un groom est un commit de plan, pas une PR) ; il est une
  **ceinture**. Un parent réutilisé (motif mika#920) qui porte déjà un `pr_url`
  a vu un dispatch `implement` aboutir : le reaper n'a rien à y faire.
- **Pourquoi aucun critère d'artefact propre au groom n'est nécessaire.** Un
  groom réussi fait re-dispatcher l'issue sur le **même parent** (`create_task`
  est idempotent sur `reference_url`), ce qui crée un sibling `pending` /
  `in_progress` — et la clause `NOT EXISTS (sibling actif)` déjà présente
  l'exclut. Inversement, un groom qui rend `GROOMED` sans que rien ne reprenne
  la main est, lui aussi, une fuite : le ramasser est correct, pas accidentel.
  Le prédicat « callback délivré, aucun sibling actif, grâce écoulée » **est**
  le critère, et il n'a pas besoin de connaître la forme de l'artefact.

Le doc-comment existant perd sa phrase « *Groom-class leak detection is a
separate follow-up (mika#1118 Option B)* », qui cesse d'être vraie dans le même
commit, et gagne la note de couplage ci-dessous.

**Couplage à documenter, pas à propager.** Le doc-comment actuel déclare une
« *Coupled pair* » avec `find_completable_parent_tasks_on_pr_url` (le versant
succès, mika#1162) et exige la symétrie de tout changement de filtre. Ce plan
rompt la symétrie **à dessein** et doit le dire : le versant succès est défini
par la présence d'un `pr_url`, qu'un groom ne produit par construction jamais.
Paramétrer le succès par la classe créerait un appel `('groom')` qui ne peut
rendre que l'ensemble vide. La note de couplage est donc réécrite en :
« *paramétrée par classe ici, volontairement pas là — un groom ne produit pas de
`pr_url` ; voir mika#2750 § 3 D1.* »

**Sites appelants de test à mettre à jour** (ajout du littéral `"implement"`,
aucun changement d'attente) : six appels dans
`crates/mika-agent/src/db/tests/reapers.rs` (l. 414, 442, 455, 486, et les deux
suivants du même module).

### D2 — Le passe-plat `AsyncDatabase`

**Fichier :** `crates/mika-agent/src/async_db.rs` (~l. 1148-1155).

Ajout du paramètre `dispatch_class: &str`, cloné dans la closure comme les
autres. Rien d'autre.

### D3 — Le reaper du moteur apprend sa seconde classe

**Fichier :** `crates/mika-agent/src/task_engine/engine.rs`.

`reap_orphaned_parent_tasks` devient `reap_orphaned_parent_tasks(&self,
dispatch_class: &str)` et est appelée **deux fois** dans la branche
`DB_SCAN_INTERVAL_TICKS` de `tick()` (~l. 1322), l'une après l'autre :

```rust
self.reap_orphaned_parent_tasks("implement").await;
// mika#2750 — la seconde classe. Les deux populations sont disjointes par
// construction (le prédicat porte sur `child.dispatch_class`), donc cet
// ordre ne coûte rien et garde toute l'échelle de réparation lisible au
// même endroit.
self.reap_orphaned_parent_tasks("groom").await;
```

Trois points internes changent, et rien d'autre :

**(a) La garde TOCTOU de mika#1126 devient classe-consciente.** Aujourd'hui elle
compare à `"implement"` en dur (`engine.rs:4342`) ; elle compare désormais à
`dispatch_class`. Sa sémantique est inchangée : « si, au moment de tuer, tous les
enfants callback délivrés sont d'une **autre** classe que celle que je balaie, je
m'abstiens ».

**(b) Le motif d'échec et le nom d'événement d'audit sont tirés d'une table
fermée**, écrite à côté de `REAPER_GRACE_SECONDS` :

```rust
/// (classe balayée, motif d'échec par défaut, nom d'événement d'audit).
/// Recensement FERMÉ : une classe absente d'ici n'est pas balayée — voir
/// le `None` ci-dessous. Les valeurs sont un FORMAT DE FIL : les opérateurs
/// groupent dessus.
const REAPER_CLASS_DISPOSITIONS: [(&str, &str, &str); 2] = [
    ("implement", "callback_delivered_without_pr_url", "task_engine_reaper"),
    ("groom", "groom_callback_delivered_without_redispatch", "groom_parent_orphaned"),
];
```

La classe `implement` conserve **mot pour mot** ses deux valeurs actuelles :
`tasks.result` et `audit_events.tool_name` sont lus par des requêtes
d'opérateur existantes, et mika#2121 (KTD5) désigne ce site comme *sole
production writer* de `callback_delivered_without_pr_url`. Renommer serait un
changement de format de fil non demandé.

La classe `groom` reçoit ses deux noms propres. `groom_parent_orphaned` est
**l'événement nommé** que l'« Attendu » n°2 réclame : il dit *quoi* (un parent de
groom), *dans quel état* (`in_progress` → `failed`) et *pourquoi* (son callback a
délivré et personne n'a repris la main).

**(c) Une classe hors table ne balaie rien.** Le lookup rend `Option`, et un
`None` sort de la fonction avec un `warn!` plutôt qu'avec un motif de repli. Un
fall-through écrirait un état terminal sous le nom d'une autre classe — la panne
exacte que le pattern `hosting_ground_truth_line` (mika#2290), cité deux fois
dans ce même fichier, existe pour interdire.

**Ce qui ne change pas et doit rester tel quel :** l'enrichissement
`parse_no_pr_reason` (qui lit la ligne ancrée `NO_PR:` du résultat de l'enfant,
mika#2121 U2) reste conditionné à la classe `implement`. Un callback de groom ne
produit pas cette ligne ; l'appeler rendrait `None` et le motif de repli serait
le motif générique de l'autre classe. Le motif groom est donc fixe.

La grâce reste `REAPER_GRACE_SECONDS` (600 s, `engine.rs:110`). Dix minutes après
qu'un callback de groom a délivré sans qu'aucun sibling ne soit actif, le vol est
mort : aucune variable d'environnement nouvelle, aucun réglage à documenter.

### D4 — Le test, qui traverse `tick()`

**Fichier neuf :** `crates/mika-agent/tests/eval/test_groom_parent_reap_2750.rs`,
plus sa ligne `mod` dans `crates/mika-agent/tests/eval.rs`.

Modèle repris de `tests/eval/test_dispatch_parent_settle_2405.rs` : base
`Database::open_in_memory()`, `AsyncDatabase::new_with_agent`, `TaskEngine`
assemblé avec un `NoopSender`, et **chaque cas traverse `TaskEngine::tick()`** —
jamais le reaper en direct. Le câblage dans la branche `DB_SCAN_INTERVAL_TICKS`
fait partie de ce qui doit rester vrai : un reaper correct mais non appelé se lit
exactement comme un reaper qui n'a rien trouvé.

Un helper sème la scène du ticket : parent `in_progress` / `self_dev` /
`manual` / `action_type='none'` / `dispatch_class='groom'` / `metadata` sans
`pr_url`, et un enfant `trigger_type='callback'` / `action_type='resume_agent'` /
`dispatch_class='groom'` / `status='delivered'`, d'`updated_at` reculé.

- **Contrôle positif** — `un_groom_dont_le_callback_a_delivre_quitte_in_progress` :
  enfant délivré il y a 700 s (> grâce), aucun sibling actif. Après `tick()`, le
  parent est `failed`, son `result` porte
  `groom_callback_delivered_without_redispatch`, et une ligne d'audit
  `groom_parent_orphaned` existe avec `before='in_progress'` /
  `after='failed'`.
- **Contrôle négatif** — `un_groom_dont_le_pilote_vit_encore_ne_bouge_pas` : même
  scène plus un sibling `in_progress`. Après `tick()`, le parent est toujours
  `in_progress` et **aucune** ligne `groom_parent_orphaned` n'a été écrite. C'est
  le « pilote vivant → rien ne bouge » exigé par le ticket.
- **Non-régression de la classe voisine** —
  `la_classe_implement_garde_son_motif_mot_pour_mot` : la même scène en
  `dispatch_class='implement'` reste reapée sous `task_engine_reaper` avec
  `callback_delivered_without_pr_url`. Sans ce troisième cas, un renommage
  accidentel de la table D3(b) passerait vert.

**Recette de vérification par injection (obligatoire, à écrire dans l'en-tête du
fichier).** Commenter la seconde ligne d'appel ajoutée en D3 dans `tick()` : le
contrôle positif doit rougir, les deux autres rester verts. Restaurer, relancer,
les trois repassent au vert. Un test qui ne rougit pas quand on retire ce qu'il
mesure ne mesure rien.

---

## Fire-Disposition

Ce plan livre **un** détecteur au sens de mika#1574 : le contrôle de
non-régression D4-3 (`la_classe_implement_garde_son_motif_mot_pour_mot`), dont le
chemin de succès est « aucun renommage du format de fil de la classe
`implement` ». Les deux autres cas de D4 fixent un comportement neuf plutôt
qu'ils ne signalent une violation.

**Disposition retenue : (a) exception nommée en allowlist — avec une allowlist
VIDE, et la raison pour laquelle elle ne peut pas ne pas l'être.**

- **Pourquoi vide.** Le détecteur épingle deux littéraux —
  `callback_delivered_without_pr_url` et `task_engine_reaper` — qui existent
  déjà en production et que le même commit laisse **inchangés** (D3(b), premier
  tuple). Il n'existe aucune violation préexistante à exempter : au moment où le
  test s'arme, l'état du dépôt est exactement celui qu'il exige. Une entrée
  d'allowlist serait une fiction.
- **Pourquoi (b) est refusée.** Livrer désarmé (`#[ignore]`) poserait un suivi
  pour armer un détecteur dont la seule violation possible serait introduite par
  **ce même commit** — c'est-à-dire précisément l'instant où il doit mordre.
- **Pourquoi (c) est refusée.** Rien à cadrer : la table D3(b) est écrite depuis
  les valeurs lues dans `engine.rs` aujourd'hui, pas depuis un état souhaité.

**Auto-nettoyage.** Les trois cas de D4 sont écrits avec les noms d'événement en
**littéraux**, jamais via les constantes de production — délibérément, et le
fichier le dit : si la table D3(b) changeait de valeur, ces assertions doivent
rougir plutôt que suivre. C'est la même discipline que
`test_dispatch_parent_settle_2405.rs:36-39` applique à `SETTLED_EVENT`.

---

## 4. Contrat de vérification

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | le parent de groom orphelin quitte `in_progress` | D4-1, via `tick()` |
| V2 | l'événement nommé existe et porte la transition | D4-1, lecture de `audit_events` |
| V3 | un pilote vivant n'est pas touché | D4-2 |
| V4 | la classe `implement` est inchangée, mot pour mot | D4-3 + les six tests existants de `db/tests/reapers.rs`, qui passent sans modification d'attente |
| V5 | le câblage dans `tick()` est réel | recette d'injection de D4 |
| V6 | rien d'autre n'a bougé | `cargo test`, `cargo clippy`, `make lint`, `scripts/verify-pipeline.sh origin/main` |

**Sonde opérateur**, à exécuter après déploiement sur l'agent `mika-dev` :

```
grep groom_parent_orphaned "$MIKA_SPIRIT_LOG_FILE"
```

Lecture : une ligne par parent de groom ramassé. **Zéro ligne en régime normal
est le résultat attendu** ; une ligne par jour sur des tickets différents est un
rail qui se répare ; la **même** issue plusieurs fois par jour est le battement
décrit au § 6, et c'est alors mika#2029 qu'il faut lire, pas ce reaper.

---

## 5. Ordre de commit

Un seul vol, trois commits, chacun compilant seul :

1. `fix(db): find_orphaned_parent_tasks prend sa classe en paramètre (mika#2750)`
   — D1 + D2 + les six sites d'appel de test. Comportement inchangé :
   l'unique appelant de production passe `"implement"`.
2. `fix(engine): le reaper de parents orphelins balaie aussi la classe groom (mika#2750)`
   — D3. C'est le commit qui change le comportement.
3. `test(eval): un parent de groom orphelin atteint un état terminal nommé (mika#2750)`
   — D4.

**Plancher :** après le commit 1, le dépôt est strictement équivalent à
aujourd'hui. Après le commit 2, le défaut est corrigé. Le commit 3 ne fait que le
prouver. Un abandon après le commit 1 ne laisse aucune dette.

---

## 6. Risques

**R1 — Le battement : crash → reap → re-promotion → crash.** Un ticket dont le
groom meurt systématiquement sera re-promu à chaque balayage ready. Le plan
n'ajoute pas de borne, et c'est délibéré : elle existe déjà en amont, dans
`crates/mika-agent/src/auto_pull.rs` (`classify_stuck_ready`,
`select_stuck_ready_candidates`, `abandon_stuck_ready`,
`enact_stuck_ready_skip`, `phase2_reconcile_stuck_ready`), qui traite les tickets
`ready` restés coincés au-delà de `MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS`.
Le choix honnête est de **mesurer avant d'ajouter** : si la sonde du § 4 montre
la même issue plusieurs fois par jour, c'est un ticket de suivi avec un compte,
pas une supposition à coder aujourd'hui. Aujourd'hui, un battement visible vaut
mieux qu'un rail vide muet pendant six heures.

**R2 — La rupture assumée de la « Coupled pair ».** D1 paramètre un versant de
la paire et pas l'autre. Le risque n'est pas le comportement (le versant succès
ne peut rien rendre pour `groom`) mais la **lecture** : un futur contributeur
pourrait propager la symétrie par réflexe. Mitigation : la note de couplage est
réécrite, pas supprimée, et nomme ce ticket.

**R3 — Le stock existant.** Au premier tick après déploiement, toutes les rows
déjà coincées — `fef8cd36` comprise — seront ramassées d'un coup. C'est l'effet
recherché, et le reaper distingue déjà ce cas : sa branche `age_hours > 24` émet
« *reaping pre-existing orphan (possible backfill from before reaper
deployment)* ». Aucun code à ajouter ; à signaler dans le corps de PR pour que la
rafale de lignes du premier tick ne se lise pas comme une panne.

**R4 — Un sibling `blocked`.** La clause `NOT EXISTS` ne retient que `pending` et
`in_progress` ; un sibling `blocked` ne protège donc pas le parent. C'est le
comportement de la classe `implement` depuis #871, inchangé ici, et il est
correct : une tâche `blocked` ne reprendra pas la main toute seule.

---

## 7. Hors périmètre

- **La cause du groom mort-né** (`end_turn` au tour 1 sans appel d'outil). Classe
  déjà nommée par mika#2029. Ce plan traite la conséquence — une row immortelle —
  pas la mort elle-même.
- **L'écrasement de `tasks.result` par un refus `webhook_dispatch_target_mismatch`**
  (mika#2649). Le ticket le déclare hors périmètre et sans effet causal.
- **Le re-drive explicite depuis le moteur**, et le durcissement structurel du
  § 3g du prompt ready-label. Justifié au § 2.1 : le relâchement du verrou atteint
  l'« Attendu » pour un prédicat, là où le re-spawn coûte un second producteur de
  dispatch.
- **Toute nouvelle borne anti-battement.** Voir R1 — conditionnée à une mesure.

---

## Definition of Done

- [ ] D1 : `find_orphaned_parent_tasks` prend `dispatch_class`, le doc-comment
      perd sa phrase « *separate follow-up (mika#1118 Option B)* » et gagne la
      note de couplage réécrite ; les six appels de `db/tests/reapers.rs` sont à
      jour et leurs attentes inchangées.
- [ ] D2 : le passe-plat `AsyncDatabase` transmet le paramètre.
- [ ] D3 : le reaper est paramétré, appelé deux fois dans `tick()`, la garde
      mika#1126 compare à la classe balayée, la table `REAPER_CLASS_DISPOSITIONS`
      est fermée et une classe absente ne balaie rien.
- [ ] D4 : les trois cas passent et la recette d'injection est écrite dans
      l'en-tête du fichier.
- [ ] `cargo test`, `cargo clippy`, `make lint`, `cargo fmt` verts.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] Corps de PR : mentionne R3 (rafale attendue au premier tick) et la sonde
      du § 4.

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria`. Les critères
ci-dessous sont dérivés de sa section « Attendu », verbatim quant au fond.

- **AC1** — Après un crash de groom, la tâche ready-label atteint en temps borné
  un **état terminal nommé** : elle quitte `in_progress` au plus tard
  `REAPER_GRACE_SECONDS` (600 s) + un tick de balayage (60 s) après que son
  callback a délivré, sans sibling actif. Vérifié par D4-1.
- **AC2** — L'auto-feeder cesse de la compter en vol : le passage à `failed` la
  sort de `idx_tasks_manual_active_ref_url`. Vérifié par la définition de l'index
  (`migrations.rs:743-747`) et par l'assertion de statut de D4-1.
- **AC3** — **Un événement nommé dit pourquoi** : `groom_parent_orphaned` en
  audit, motif `groom_callback_delivered_without_redispatch` dans
  `tasks.result`. Vérifié par D4-1.
- **AC4** — **Contrôle négatif** : un pilote vivant (sibling `pending` ou
  `in_progress`) laisse le parent intact et n'écrit aucun événement. Vérifié par
  D4-2.
- **AC5** — Aucune régression sur la classe `implement` : motif et nom
  d'événement inchangés mot pour mot. Vérifié par D4-3 et les tests existants.
- **AC6** — Aucun humain n'a à lire un journal pour que l'un des deux états soit
  atteint : le réparateur est câblé dans le balayage périodique du moteur, et la
  recette d'injection de D4 le prouve.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/tasks.rs` (paramètre + doc-comment réécrit) | 28 |
| `crates/mika-agent/src/db/tests/reapers.rs` (six sites d'appel) | 6 |
| `crates/mika-agent/src/async_db.rs` (passe-plat) | 8 |
| `crates/mika-agent/src/task_engine/engine.rs` (table fermée, reaper paramétré, 2e appel dans `tick()`) | 56 |
| tests (`crates/mika-agent/tests/eval/test_groom_parent_reap_2750.rs` + `mod` dans `eval.rs`) | 175 |

Total estimé : 273 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 273 < 279 et 273 < 1000 : les deux
bornes passent, la plus serrée de six lignes. La marge est mince et elle est
réelle, parce que le volume est dominé par un seul poste — le fichier de test —
et que les quatre autres sont des éditions chirurgicales dont on connaît les
lignes exactes (un paramètre, un passe-plat, une table de deux tuples, un appel).
Le travail de production proprement dit pèse 98 lignes.

Le poste qui pourrait déborder est D4. Sa borne de sécurité est écrite dans le
plan plutôt que laissée au jugement du pilote : **trois cas, un helper de
semis, aucun second helper**. Si le fichier dépasse nettement 175 lignes, c'est
que les cas se sont mis à construire chacun leur scène — le remède est de
factoriser le semis, jamais de retirer le contrôle négatif ou la non-régression,
qui sont l'un et l'autre exigés par le ticket et par la Fire-Disposition.
