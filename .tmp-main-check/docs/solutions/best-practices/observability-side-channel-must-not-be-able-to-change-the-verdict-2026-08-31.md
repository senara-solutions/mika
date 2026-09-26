---
module: milestone_manager
tags: [observability, audit-events, auth-boundary, fire-and-forget, signature-as-guard, false-positive-suppression]
problem_type: bug-class-prevention
category: best-practices
---

# Un canal d'observation ne doit pas POUVOIR changer le verdict qu'il observe

## Problème

mika#1949 (Porte 3) ajoute un registre : chaque échec d'authentification
inter-frontières écrit une ligne `audit_events`. Le canal est greffé sur un
chemin de refus — c'est-à-dire sur du code dont le seul travail est de décider
et de renvoyer une réponse.

Deux modes d'échec guettent ce genre de greffe, et ils sont symétriques :

1. **Le registre casse la décision.** Un `record()` qui retourne `Result` finit,
   un jour, derrière un `?`. Le refus devient alors un 500, ou pire : une
   requête qui aurait dû être refusée passe parce que l'écriture d'audit a
   échoué en premier. La fonctionnalité d'observabilité a créé un mode de panne
   que le code n'avait pas.

2. **Le registre devient illisible.** Le plan disait « jeton absent → `Missing`,
   vide → `Empty` ». Appliqué inconditionnellement, ce test tire sur chaque
   livraison **réussie** vers un endpoint qui n'exige aucun bearer — une
   configuration parfaitement valide. Le registre se remplit alors d'alarmes qui
   ne veulent rien dire, et un registre que personne ne lit est pire que pas de
   registre : il ressemble à de la couverture.

## Solution

**Pour (1) : que la signature interdise la propagation, pas la convention.**

```rust
pub trait AuthBoundaryLedger: Send + Sync {
    /// Record the failure. Never blocks the caller, never fails visibly.
    fn record(&self, err: AuthBoundaryError);   // <- pas de Result, pas de Future
}
```

`record` retourne `()`. Il n'y a **aucune** erreur qu'un appelant puisse
accidentellement propager, et **aucun** futur qu'il puisse accidentellement
attendre. L'implémentation de production détache l'écriture (`tokio::spawn`) et
journalise en `warn` si elle échoue. Un commentaire disant « n'utilisez pas le
`?` ici » aurait tenu jusqu'au premier refactor ; une signature tient toujours
(cf. `prompt-enforcement-structural-guards.md`, même famille).

Le test qui compte n'argumente pas la propriété, il **injecte la panne** : le
registre pointe sur un `agent_id` absent de la table `agents`, l'INSERT viole la
clé étrangère, et on assère d'abord que l'écriture échoue vraiment — sans quoi
le test ne prouve rien — puis que l'appelant revient normalement.

**Pour (2) : ne classer un symptôme qu'à l'endroit où il discrimine.**

Absent et vide ne sont examinés **que** sur un refus effectif : « l'autre côté
nous a refusés, et la raison est que nous n'avons rien présenté / présenté du
vide / présenté quelque chose qu'il n'a pas accepté ». Un succès ne produit
aucune ligne. Une panne non-auth (500, corps malformé) non plus, et un test
nommé le dit à voix haute.

Corollaire utile : une variante d'énumération que cette frontière ne peut
**pas** produire (`Invalid`, qui suppose un contrôle de forme que ce site
n'applique pas) est épinglée par un test qui l'exclut — pour que l'absence se
lise comme une décision et non comme un oubli.

## Trois pièges rencontrés au passage

- **Un numéro de ligne cité dans un plan est une affirmation à revérifier.** Le
  plan désignait `db.rs:2338` comme « l'écrivain `audit_events` » ; cette ligne
  est en réalité une migration v8→v9. Le vrai écrivain est
  `evidence/audit.rs::log_audit_event`. Grep le symbole, jamais la ligne.

- **Une tâche de fond n'a pas forcément de poignée sur ce dont elle a besoin.**
  La cadence du manager est spawnée avec `(cfg, cancel, token_resolver)` et rien
  d'autre : aucune base de données. Injecter `Option<Arc<dyn Ledger>>` — comme
  le module injecte déjà `AuthAlarmSink`, `ReportDeliverer`, `TokenResolver` —
  garde les sites de frontière testables sans base et laisse `None` être une
  configuration valide, pas une panne.

- **Élargir une signature publique coûte des tests ; un contexte, non.** Passer
  d'un `run_manager_cycle_with(cfg, state, deliverer, now)` à six paramètres
  aurait réécrit une douzaine de tests pour rien. Un `CycleContext` avec des
  `with_*` par défaut à `None` ajoute le câblage sans toucher l'existant.

## Vérification

- `cargo test -p mika-common auth_boundary` — 4 tests, dont le contrôle négatif
  « ne rend jamais une valeur qu'on ne lui a pas donnée ».
- `cargo test -p mika-agent --lib -- milestone_manager auth_boundary evidence::audit`
  — 145 tests, dont la porte structurelle `no_dispatch_test`.
- `git diff main -- crates/mika-agent/src/milestone_manager/no_dispatch_test.rs`
  vide : le registre n'accorde aucune autorité d'écriture au manager.

## Voir aussi

- `prompt-enforcement-structural-guards.md` — la même doctrine, appliquée aux
  prompts plutôt qu'aux signatures.
- `docs/solutions/logic-errors/2013-token-resolved-once-at-spawn-freezes-a-renewable-credential.md`
  — mika#2013, le précédent mesuré de la panne silencieuse d'identifiant que ce
  registre rend visible.
- `mika-platform/docs/operator/token-rotation-procedure.md` — la moitié
  opérateur du même travail.
