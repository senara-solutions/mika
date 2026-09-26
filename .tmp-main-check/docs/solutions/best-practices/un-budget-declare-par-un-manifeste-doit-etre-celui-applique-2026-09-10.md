---
title: Un budget déclaré par un manifeste doit être celui qu'on applique — sinon c'est une croyance
date: 2026-09-10
last_updated: 2026-09-10
category: best-practices
module: mika-agent/agent_loop
problem_type: best_practice
component: dev-loop
severity: high
applies_when:
  - Un manifeste (skill.toml, package.json, une CRD) déclare une limite par unité
  - Le moteur agrège ces limites — max, somme, dernier gagnant — avant de les appliquer
  - Un prompt, une doc ou un commentaire cite la valeur déclarée comme si elle était tenue
  - Un tour d'agent meurt sur son enveloppe sans que rien n'ait l'air d'avoir dépassé
---

# Un budget déclaré par un manifeste doit être celui qu'on applique

## Le problème

Le 2026-09-09, mika-qa a tourné **quatre fois** sur la PR #2275 sans jamais poser
de verdict. Chaque tour notifiait Telegram — donc chaque tour avait bien tourné —
et la PR restait muette. Le ticket mika#2276 a d'abord listé trois hypothèses :
mauvaise identité du poste, exception avalée sur le `gh pr review`, format de
verdict non reconnu. **Les trois étaient fausses.** Le verdict n'était jamais
produit.

Trace `921f11f0-acd4-11f1-8bc6-90c3b908c45a` :

| Instant (UTC) | Événement |
|---|---|
| 05:01:23 | le tour a lu le callout de plan et le diff complet |
| 05:01:32 | `run_shell` → `cargo test --release … test_reaper_reaps_live_pending_pilot_2272` |
| 05:05:30 | rend la main après **237,9 s** — un second `cargo test --release` part |
| 05:09:21 | `agent deadline exceeded`, `steps_completed=5` — aucun verdict |

469 s des ~506 s d'enveloppe mangés par deux appels d'outil.

La lecture évidente s'arrête là : « la QA recompile dans son budget ». Elle est
exacte et elle s'arrête un cran avant la cause. **Pourquoi une recompilation
a-t-elle pu vivre 238 s dans un outil dont le manifeste déclare 30 s ?**

## La cause

`shell-exec/skill.toml` déclare `timeout_secs = 30`, et `run_shell` est son outil.
Mais le moteur ne lisait pas ce chiffre-là. Il calculait **un seul** budget pour
tout le tour :

```rust
// agent_loop/mod.rs — avant mika#2276
fn max_skill_timeout(matched: &[&SkillEntry], provider: &str, model: &str) -> u64 {
    matched.iter().map(|e| e.effective_timeout(provider, model)).max()
        .unwrap_or(TOOL_TIMEOUT_SECS)
}
```

…et l'appliquait **uniformément à chaque appel d'outil**. `qa-review` déclare
`dependencies = ["github", "build-mika"]` ; `build-mika` déclare 300 s. Donc dans
un tour de revue QA, `run_shell` disposait de 300 s — parce qu'un *autre* skill,
chargé pour une *autre* raison, était plus généreux.

Le prompt de qa-review écrivait, à la ligne 194 :

> Measured cost on `mika` (3323 tracked files): ~0.6s, against `run_shell`'s **30s budget**.

La doctrine raisonnait sur un plancher que le moteur n'a jamais offert. Ce n'est
pas une phrase inexacte à corriger : c'est le seul endroit du système où la règle
était écrite, et elle n'avait aucun appui.

## Ce qui rend cette classe difficile à voir

**Le budget agrégé est invisible depuis les deux bouts.** Le mainteneur de
`shell-exec` lit 30 dans son manifeste ; le mainteneur de `build-mika` lit 300
dans le sien. Aucun des deux ne lit « 300 s pour `run_shell` », parce que ce
chiffre n'est écrit nulle part — il naît d'un `max()` sur une liste construite au
moment du tour, à partir d'un graphe de dépendances de skills.

**Un détail qui aurait dû alerter et qui n'alerte personne :** les six skills
aux plus gros budgets déclarés — `dev-pilot` (600), `dev-groom` (600),
`address-pr-comments` (600), `resolve-pr-conflicts` (600), `build-mika` (300),
`deploy-mika` (120) — n'exposent **que des outils `long_running`**, lesquels
court-circuitent l'application du timeout (spawn détaché, callback). Autrement
dit : **leurs budgets ne protégeaient jamais leurs propres outils.** Leur unique
effet observable était de relever le plafond des outils des *autres* skills. Une
valeur dont le seul effet est un effet de bord sur autrui est une valeur que
personne ne relit.

**Et le dépassement était silencieux là où il comptait.** Sur dépassement,
`persist_deadline_fallback` persiste un message générique — *« I'm sorry, that
took too long »* — que le call-site envoie sur le canal de réponse comme
n'importe quelle vraie réponse. `AgentOutput` ne portait aucun champ disant
*pourquoi* le tour s'était terminé. « Le tour a abouti » (Telegram) et « le tour
a conclu » (verdict) étaient deux faits différents que rien dans le code ne
séparait — ce qui rendait le filet impossible à écrire au call-site, et pas par
oubli.

## La règle

> **Une limite déclarée par unité doit être appliquée à cette unité.** Si le
> moteur agrège des limites déclarées avant de les appliquer, la valeur du
> manifeste cesse d'être un contrat et devient une indication décorative — et
> toute doctrine qui la cite devient fausse sans que rien ne le signale.

Trois corollaires, chacun payé par ce ticket :

1. **Une interdiction écrite dans un prompt, contredite par le substrat, tient
   jusqu'au jour où un modèle prend la porte que le moteur a laissée ouverte.**
   Le prompt QA disait *déjà* 30 s. La classe est celle de
   `feedback_prompt_enforcement_fragile` : la réparation n'est pas de réécrire la
   phrase, c'est de rendre la phrase vraie.
2. **Une valeur qui n'a d'effet que sur autrui n'est jamais relue.** Chercher
   dans un système les déclarations dont le seul effet observable est indirect :
   ce sont les candidats à la dérive silencieuse.
3. **Un budget rendu porteur doit être réécrit sciemment.** En fermant M1 on a
   déclaré explicitement `timeout_secs = 30` dans `qa-review/skill.toml` — la
   même valeur que le défaut, mais désormais une décision plutôt qu'un défaut que
   personne n'avait lu.

## Le geste

**M1 — le budget suit son skill.** `build_skill_tool_timeouts` construit une
carte `nom d'outil → budget du skill qui le définit`, à côté de
`build_skill_tool_map` et `build_skill_data_grades`, depuis le même `matched`,
sous la même règle de collision (dernier gagnant). Les trois cartes bougent
ensemble ou un outil serait dispatché vers le handler d'un skill et coupé au
budget d'un autre. `max_skill_timeout` garde sa sémantique de maximum et devient
un repli ; ce qu'il cesse d'être, c'est le budget par outil.

**Le fan-out compte.** `run_loop` a **trois** appelants — conversation, silent,
team — et les trois calculaient leur `skill_timeout`. Un fix qui n'en couvre que
deux est un fix qui ment (`feedback_structural_gate_audit_grep_all_callsites`).

**M2 — le dépassement parle sur le canal qui manquait.** `AgentOutput` porte
`deadline_exceeded: Option<DeadlineOverrun>`, stampé au seul endroit par lequel
tous les chemins de deadline repassent (`persist_deadline_fallback`), donc
impossible à oublier sur l'une des trois portes. Le call-site en tire un
`VERDICT: hold[review]` posté sur la PR — forme existante, comprise par
`verdict_handler`, sans nouvelle branche ni gate CODEOWNERS.

## Le contrôle négatif, et pourquoi il n'était pas une formalité

Le premier jet du test d'AC4 aurait pu passer sur `main` par coïncidence : sur un
jeu où le skill le plus court est *aussi* le plus généreux, `max()` rend la bonne
valeur. Le contrôle négatif — faire rendre à la fonction la sémantique de `main`
et vérifier le rouge — a produit le chiffre du défaut :

```
assertion `left == right` failed: run_shell doit être coupé au budget déclaré
par shell-exec (30 s), pas au maximum du tour
  left: Some(300)
 right: Some(30)
```

Et sur M2, la même manœuvre a produit `NotApplicable("turn_completed")` : zéro
POST, PR muette — le symptôme du ticket, reproduit à la demande.

## Voir aussi

- `docs/solutions/architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md`
  — l'autre face : un lecteur plus strict que son producteur.
- `docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md`
  — même famille : un mécanisme qui « existe » dans le code sans l'appui qui le
  rendrait effectif.
- `docs/solutions/best-practices/un-booleen-qui-devient-un-seuil-a-plus-de-lecteurs-que-vous-ne-croyez-2026-09-05.md`
  — même famille : une valeur dont les lecteurs réels dépassent ceux qu'on croit.
