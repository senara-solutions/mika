---
module: skills/bundled/_shared/dispatch-lib
tags: [dev-loop, claude-pilot, dispatch-lib, callback-contract, diagnostic-honesty, guardrail, idle-timeout, no-pr-reason, halt-subtype, mika-2029, mika-2121, mika-2149, mika-2539, claude-pilot-177]
problem_type: silent-wrong-answer
category: dev-loop
---

# Le défaut d'un instrument de panne n'est pas le silence, c'est une réponse plausible

## Problème

Un pilote tué par un garde-fou avant d'avoir produit quoi que ce soit —
`status: terminated`, HEAD immobile, arbre propre — remontait à mika-dev par deux
canaux, et **les deux rendaient une réponse fausse mais crédible**.

**Le canal machine.** mika#2121 a déclaré un contrat *total* : « every delivered
callback carries exactly one `PR:` or `NO_PR: <reason>` line »
(`dispatch-lib.sh:2317`). Le chemin `_terminated_empty` de `_run_claude_pilot`
est le seul qui saute `_post_flight_recovery`, et les deux sites qui posent cette
ligne vivent dedans. `_classify_terminated_session` n'en posait aucune : sa
sortie portait `Halt:`, `Halt class:`, `Retry hint:`, `Outcome:` — et rien
d'autre. Faute de ligne à parser, `reap_orphaned_parent_tasks`
(`task_engine/engine.rs`) écrivait son motif de repli,
`callback_delivered_without_pr_url`, dans `tasks.result`. Ce motif existe pour
une **autre** panne : un callback malformé. La population « pilote tué avant
d'avoir produit » (8/8 depuis le 2026-09-04) atterrissait donc dans un seau qui
accusait le messager, et toute requête sur le taux de l'une mesurait la somme des
deux.

**Le canal humain.** Le même callback portait, dans son texte livré, une chaîne
causale affirmée :

```
Anthropic 401 / SDK stall → guardrail idle_timeout 300s → pilot dies at Turns:1
```

Elle avait été **réfutée le 2026-08-29** : les journaux d'egress montrent
`POST /v1/messages -> 200` sur les sessions concernées, et six lignées candidates
ont été éliminées une à une. La cause réelle était ailleurs et dans un autre
dépôt — `content_block_stop` du bloc `tool_use` courant refermait la fenêtre
d'attente-modèle que le tool result du même bloc venait d'ouvrir, ramenant un
budget de 900 s à 300 s pendant un TTFT bimodal (~355 s dans son mode lent) :
**claude-pilot#177, fermé le 2026-09-11**. Pendant six semaines, le callback a
envoyé l'opérateur vers `mika#1901` — une lignée d'egress CLOSED, réfutée, et
dont la correction n'aurait rien changé.

## Cause racine

Les deux moitiés sont le même défaut sous deux formes.

**Un repli n'est pas une absence.** Un parseur qui ne trouve pas sa ligne a deux
sorties possibles : refuser, ou choisir une valeur par défaut. Ici le défaut est
un motif déjà porteur de sens, donc son écriture est indiscernable d'une mesure.
Le trou ne se signale nulle part : la colonne est pleine, la requête rend un
nombre, et le nombre est faux. Un contrat déclaré *total* qui garde un chemin non
couvert ne produit pas un trou visible — il produit une **mauvaise classe**.

**Un diagnostic livré est une affirmation causale, pas un commentaire.** Un
commentaire périmé coûte la lecture de celui qui le croit. Le même texte envoyé
dans un callback coûte l'enquête de celui qui le reçoit, et il la coûte à chaque
occurrence. Rien dans le format « cause → symptôme → forme observée » ne porte sa
date ni son état de réfutation : la chaîne restait aussi affirmative le jour de
sa réfutation que le jour de son écriture.

## Solution

**Un motif nommé, dérivé du sous-type déjà résolu.** Une fonction unique
transforme le sous-type de halt en motif consommable :

```sh
_no_pr_reason_from_halt_subtype() {
    local s="${1:-}"
    [ -n "$s" ] || { printf '%s' "pilot_terminated_unknown"; return 0; }
    s=$(printf '%s' "$s" | tr '[:upper:]' '[:lower:]' | tr -c 'a-z_' '_')
    printf '%s' "pilot_terminated_${s}"
}
```

et la branche `_terminated_empty` la pose via `_set_pr_status_line`, qui retire
toute ligne `PR:`/`NO_PR:` préexistante avant d'ajouter la sienne — « exactement
une » devient vrai par construction plutôt que par convention.

**Le sous-type, pas la famille.** `_halt_family` (mika#2149) agrège
`error_max_turns` et `error_max_budget_usd` en `budget_exhausted`, et
`stall_detected`/`empty_response` en `model_unproductive`. Agréger effacerait
exactement la distribution que le motif existe pour mesurer. Le prix est un motif
de 46 caractères (`callback_no_pr_pilot_terminated_error_max_turns`) dans une
colonne TEXT sans contrainte de longueur : un prix, pas une limite.

**Normaliser la forme, parce que le consommateur tronque au lieu d'échouer.**
`dispatcher.rs::RE_NO_PR` vaut `(?m)^NO_PR:\s+([a-z_]+)`, greedy et non ancré à
droite. Un sous-type `Error-Max-Turns` n'y échoue pas : il **capture
`pilot_terminated_error`** — plausible, agrégeable, faux. Les sous-types amont
viennent d'un `Literal` Python que ce dépôt ne contrôle pas, donc la correction
porte sur la *forme* de ce qu'on émet, pas sur une liste de valeurs attendues. Et
une queue vide est traitée comme absente (`pilot_terminated_unknown`) : sans ça
`[a-z_]+` ne matcherait pas, la ligne serait ignorée, et le kill retomberait dans
le motif générique — le trou refermé par son propre correctif.

**Le diagnostic réfuté est retiré des deux sites et promu ici.** Le commentaire
de `_run_pilot_sandboxed` garde son sujet réel — pourquoi
`CLAUDE_CODE_API_BASE_URL` est posé à côté de `ANTHROPIC_BASE_URL` — et perd la
conséquence réfutée, sous un en-tête qui dit explicitement *ce que ce commentaire
ne prétend plus*. Le texte livré renvoie à la ligne `Halt class:` que la même
sortie porte déjà, et nomme claude-pilot#177.

## Prévention

- **Le repli d'un parseur ne doit jamais être un motif porteur de sens.** Si le
  repli doit exister, il se nomme `unknown` : une requête peut alors distinguer
  « pas mesuré » de « mesuré comme ceci ».
- **Un contrat déclaré total se vérifie par énumération de ses chemins**, pas par
  le taux de ceux qu'on a vus passer. La justification du motif nommé ici est
  structurelle : octobre a mesuré 0 `idle_timeout` sur 99 sessions, mais au taux
  de fin septembre (6/489 ≈ 1,2 %) observer zéro a une probabilité d'environ
  0,30. **L'extinction n'est pas établie**, et elle n'avait pas à l'être : le
  contrat en devrait une ligne même si la classe s'éteignait demain.
- **Un diagnostic livré à un agent se réfute au même endroit qu'il s'affirme.**
  Le détecteur qui tient ça ici est un scan de source (`Anthropic 401` absent de
  `dispatch-lib.sh`) **doublé de sa contre-vacuité** : la ligne légitime
  `401 on api.github.com` (le vrai 401 GitHub, mika#2056) doit **rester**
  présente. Sans ce second côté, un `sed` trop large satisfait le premier.

### Deux pièges d'implémentation, à ne pas refaire

**La substitution de commande est un sous-shell, et le mémo meurt avec lui.**
`_resolve_halt_subtype` (mika#2539) est le résolveur unique memoïsé : il scrape
stderr une fois et publie `_HALT_SUBTYPE_RESOLVED`. La branche fait
`RESULT=$(_classify_terminated_session)` — un sous-shell. Appeler le résolveur
*dedans* publierait le mémo dans un processus qui s'achève, et chaque motif
vaudrait `pilot_terminated_unknown` : un bug de la classe exacte que le correctif
ferme. Il est donc appelé **en commande, avant la substitution**, que le
sous-shell *hérite* : la bannière `Halt class:` et le motif `NO_PR:` décrivent la
même résolution par construction. L'ordre est épinglé par une sonde, calculée sur
la vue sans commentaires de la branche — scanné sur la source brute, il comptait
les identifiants cités dans sa propre prose.

**Un accent grave dans un `printf '%s' "…"` est une substitution, pas une
citation.** La première rédaction du texte livré disait
`` The `Halt class:` line above is what names it ``. Bash a exécuté `Halt class:`
comme une commande ; le texte livré lisait `The  line above is what names it`, et
l'erreur est partie sur un fd 2 déjà pointé sur `/dev/null` (mika#903) — échec
parfaitement silencieux. Corrigé en guillemets simples, et verrouillé par un scan
de forme : zéro accent grave dans le corps non commenté de
`_classify_terminated_session`.

## Références

- `skills/bundled/_shared/dispatch-lib.sh` — `_no_pr_reason_from_halt_subtype`,
  la branche `_terminated_empty` de `_run_claude_pilot`,
  `_classify_terminated_session`, `_resolve_halt_subtype`, `_halt_family`
- `skills/bundled/_shared/test-dispatch-lib.sh` — section mika#2029 (sondes
  V0, V1a–V1f, V2a–V2c, V3, V4) et le scan mika#2539 S4
- `crates/mika-agent/src/task_engine/dispatcher.rs` — `RE_NO_PR`,
  `parse_no_pr_reason`
- `crates/mika-agent/src/task_engine/engine.rs` — `reap_orphaned_parent_tasks`,
  seul producteur de `callback_no_pr_*` / `callback_delivered_without_pr_url`
- senara-solutions/claude-pilot#177 — la cause, côté pilote, fermée le 2026-09-11
- senara-solutions/mika#2029 (ce ticket, vol 1), mika#1772 (le rapport du kill),
  mika#2035 (les AC), mika#2121 (le contrat total), mika#2149 (`_halt_family`),
  mika#2539 (le résolveur unique), mika#1901 (la lignée d'egress réfutée)
