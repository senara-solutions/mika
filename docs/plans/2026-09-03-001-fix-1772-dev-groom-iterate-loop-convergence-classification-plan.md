---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
type: fix
issue: senara-solutions/mika#1772
created: 2026-09-03
---

# fix(loop-substrate) : classer ce qui reste de mika#1772 — la convergence de `_iterate_groom_loop` quand le plan, lui, existait

## Goal Capsule

- **Objective.** Fermer mika#1772 sur son seul reste réel : l'incident du 2026-07-04 sur mika#1723,
  où un plan **existait** sur la branche et où `_iterate_groom_loop` a tout de même rendu non-zéro,
  alors qu'un `/mika-ask-arch` manuel a convergé `GROOMED` en une passe le même jour.
- **Means.** Trois gestes, dans cet ordre : (1) constater par sonde à double contrôle que la preuve
  historique demandée par le corps du ticket **n'existe pas**, et le dire ; (2) classer la cause par
  réfutation datée sur l'historique du code plutôt que par archéologie de logs ; (3) verrouiller la
  classe par un test de régression qui échoue sans le correctif, et prouver en vol que la boucle
  converge.
- **Authority.** Le corps du ticket **et ses deux commentaires opérateur du 2026-08-28** (mika#2140 :
  les commentaires sont le ticket). Là où l'un et les autres divergent, le commentaire postérieur
  prime. Là où le ticket et l'historique du dépôt divergent sur un fait datable, l'historique prime —
  le corps l'autorise en écrivant `A/B/C/**other**` dans son critère de succès.
- **Stop conditions.** Remonter à l'opérateur si la classification exige de modifier le contrat de
  convergence architecte au-delà d'un site de `return 1` (`_arch_ask`, `_parse_disposition`,
  `_parse_verdict`, `_write_canonical_callout`). Remonter si AC4 (vérification en vol) échoue pour
  une cause étrangère à la boucle : ce serait un ticket de substrat distinct, pas une rechute de
  #1772.
- **Execution profile.** Bash uniquement — `skills/bundled/_shared/dispatch-lib.sh` et
  `skills/bundled/_shared/test-dispatch-lib.sh`. Aucun changement Rust attendu.
- **Tail ownership.** PR sur `fix/1772/loop-substrate-dev-groom-iterate-groom`. `Closes #1772` si
  AC1–AC5 tiennent ; `Refs #1772` sinon, avec le reste nommé dans un ticket de suivi.

---

## Product Contract

### Summary

mika#1772 a été ouvert le 2026-07-13 comme « investigation A/B/C » sur un échec de convergence. Sept
semaines plus tard, deux de ses trois candidats sont réfutables sans investigation, son troisième est
livré, et sa procédure d'investigation repose sur une source qui ne contient pas ce qu'elle promet.
Ce plan retire ce qui est mort, nomme ce qui reste, et le ferme.

### Problem Frame

#### Ce que le ticket demandait, et où en est chaque morceau

| Élément du ticket | État au 2026-09-03 | Preuve |
|---|---|---|
| Candidat **C** — « escalate-with-diagnostic » | **livré** | PR#2028, mergé 2026-08-29T00:18:50Z, squash `b84fdbc8` sur `main`. `_groom_warn` pose `GROOM_LOOP_FAILURE_REASON` sur les 19 sorties `return 1` de la boucle ; `dispatch_claude_pilot` émet la raison mesurée. Verrouillé par le bloc `mika#1772 — an honest dev-groom callback` de `test-dispatch-lib.sh:3524`. |
| Commentaire opérateur, item **1** — « pourquoi dev-groom ne commite aucun plan » | **diagnostiqué ailleurs, corrigé** | mika#2141 : le bac à sable ne montait que `$WORKTREE_DIR`, donc le `.git` du worktree (un *fichier* `gitdir: …` pointant hors du namespace) était introuvable. Introduit par `e4f24677` (2026-08-04), corrigé par PR#2146 mergé 2026-09-02T11:42:30Z. Les branches vides de mika#2013 et mika#1772 constatées le 2026-08-28 sont cet effet-là, pas un défaut de convergence. |
| Commentaire opérateur, item **2** — « pourquoi il annonce ensuite *Plan exists on branch* » | **livré** | même PR#2028. |
| Candidat **A** — « la session second-pass ne référence pas la first-pass » | **réfuté** | voir § *Ce que le grooming a déjà réfuté*. |
| Candidat **B** — « la boucle rejette les paraphrases connues de l'architecte » | **réfuté** | idem. |
| Étape d'investigation 1 — « `grep` sur `server.log` … 14 hits » | **prémisse fausse** | idem. |
| Critère de succès (a) — cause classée avec preuve `server.log` | **reste, sous une autre preuve** | AC1/AC2. |
| Critère de succès (b) — test de régression sur le pas précis | **reste** | AC3. |
| Critère de succès (c) — `dev-groom` ne rend plus non-zéro sur cette classe | **reste, non mesuré** | AC4. |

#### Ce que le grooming a déjà réfuté

Trois constats, datés, obtenus pendant le grooming du 2026-09-03. Ils ne ferment pas le ticket — ils
retirent trois voies mortes du chemin de l'implémenteur.

**1. Candidat A réfuté par la date.** Le fil de session vers la seconde passe n'a jamais manqué :
`_arch_ask "mika-arch-second-review" "$plan_path" "$session_id"` existe depuis `1eb5a034`
(2026-05-25, *iterate-loop primitives in dispatch-lib*, mika#1271) — **40 jours avant** l'incident du
2026-07-04. Le mécanisme que le candidat A propose d'ajouter était déjà là quand l'incident a eu lieu.

**2. Candidat B réfuté par la date.** La tolérance aux paraphrases est le parsing flou deux-paliers de
`00c73aa2` (2026-05-27, *two-tier fuzzy disposition parsing for iterate-loop*, mika#1272) — **38 jours
avant** l'incident, avec son témoin `_disposition_was_fuzzy` qui annote la trace. Là encore, présent
au moment des faits.

**3. L'étape d'investigation 1 vise une source vide.** Sonde à double contrôle sur `/var/log/mika/server.log`
(18 Go), le 2026-09-03 :

| motif | occurrences | nature |
|---|---|---|
| `ready_label_engine_dispatched` (contrôle positif) | **1650** | évènements structurés du moteur — la source est bien lue |
| `invoking mika-arch first-pass` / `converged on GROOMED for` | **6** | les six sont des `"message":"llm request body"` où un agent **cite le code** — aucune n'est une ligne de stderr |
| `iterate_groom_loop` (toutes formes) | ≫ 30 | toutes en corps de requête/réponse LLM (plans, comptes rendus, prompts) |

`server.log` ne reçoit **aucun** stderr de `dispatch-lib`. Et le fichier le dit de lui-même :
`dispatch-lib.sh:1387` — « the stderr echo … does NOT land in `/var/log/claude-pilot/<id>.stderr`
(that file captures only the later claude-pilot subprocess stderr) ». Le stderr du handler n'a pas de
puits durable.

**Conséquence directe :** pour l'incident du 2026-07-04, le worktree est détruit (donc
`$WORKTREE_DIR/.claude/groom-verdict-trail.log` avec lui), `GROOM_LOOP_FAILURE_REASON` n'existait pas
encore, et aucun log ne porte la ligne. `tasks.result` de `e781006e-…` est **vide** ; la tâche est
`cancelled`. **La preuve demandée par le critère (a) n'existe plus.** Un implémenteur qui suit l'étape
1 telle qu'écrite brûlera ses tours à chercher dans 18 Go quelque chose qui n'y a jamais été.

#### Ce qui reste, alors

Une seule question, et elle est étroite : **parmi les 19 sorties `return 1` de `_iterate_groom_loop`,
laquelle a tiré le 2026-07-04, et cette sortie est-elle encore atteignable aujourd'hui ?**

Deux d'entre elles ont été neutralisées depuis, sans que personne l'ait rattaché à #1772 :

- `f8b63530` (2026-07-25, mika#1823) enveloppe la première passe dans un ré-essai borné sur
  `Disposition:` non parsée — 1 tentative + 1 reprise, session préservée. C'est la classe
  « l'architecte a répondu mais sans la ligne de verdict ».
- PR#2028 (2026-08-29) n'a pas changé les sorties, mais les a rendues **nommées** : la prochaine
  occurrence portera sa raison jusque dans `tasks.result`, qui est durable en base.

L'hypothèse de travail que ce plan met à l'épreuve, et qu'il faut pouvoir **infirmer** :
> la sortie qui a tiré le 2026-07-04 est celle de la disposition non parsée, déjà fermée par mika#1823
> le 2026-07-25 ; le ticket est donc résolu par un correctif adjacent, et ce qui manque est la preuve,
> pas le code.

Le contrôle qui l'infirmerait : une occurrence de la classe **postérieure** au 2026-07-25 portant une
raison autre que `first-pass disposition UNPARSED`. Ce plan la cherche en base avant de conclure
(§ Phase 1, pas 3).

### Non-goals

- Ne rejoue pas mika#2141 ni la voie egress/auth du pilote. Une session tuée par `idle_timeout` ou
  par un `policy:deny` n'est pas un défaut de convergence — PR#2028 la classe déjà comme telle.
- Ne modifie pas le contrat architecte (`mika-arch-groom-ticket`, `mika-arch-second-review`, la
  grammaire `Disposition:` / `Verdict:`).
- N'ajoute pas de troisième passe architecte. Deux passes maximum reste la règle.
- Ne touche pas au chemin `auto_skipped` / `already_groomed`, même si c'est lui qui empêche
  aujourd'hui de mesurer la classe en production (voir Risque R2).

---

## Phases

### Phase 1 — Établir ce qui est encore mesurable (investigation, sans changement de code)

1. **Reconduire la sonde à double contrôle** sur `server.log`, dans le même appel, et consigner les
   trois compteurs du tableau ci-dessus. Le contrôle positif (`ready_label_engine_dispatched`) doit
   être non nul, sinon la sonde ne prouve rien.
2. **Constater l'absence de la trace 2026-07-04** : `tasks.result` de `e781006e-ade5-445d-83a3-dc1d005e8288`
   et de `f490c32f-6aa0-4942-b10f-c3e13204f75e` ; existence de
   `$WORKTREE_DIR/.claude/groom-verdict-trail.log` pour la branche de mika#1723 ; existence de la
   branche `*1723*` sur `origin`. Écrire le résultat, y compris s'il est « rien ».
3. **Chercher le contrôle qui infirme l'hypothèse de travail.** Requête sur `~/.mika/data/mika.db` :
   toute tâche postérieure au 2026-07-25 dont `result` contient `_iterate_groom_loop` ou une raison
   `_groom_warn`. Classer chaque occurrence par sa raison.
   - **Si une occurrence post-2026-07-25 porte une raison autre que `UNPARSED`** → l'hypothèse tombe ;
     cette raison devient la cible du correctif, et la Phase 2 la traite.
   - **Si aucune n'existe** → l'hypothèse tient au titre de meilleure explication disponible, et la
     Phase 2 se réduit à la verrouiller.
4. **Lire les 19 sites `return 1`** de `_iterate_groom_loop` et en dresser la table : site, raison
   posée par `_groom_warn`, condition d'atteinte, et si mika#1823 ou PR#2028 l'a modifiée.

*Livrable :* la table des 19 sorties + la classification datée. Aucune modification de code.

### Phase 2 — Traiter la sortie identifiée

Deux branches, pré-spécifiées pour qu'aucune ne se décide au moment où elle arrive.

**Branche α — une raison non-UNPARSED est trouvée en base (pas 3 infirme l'hypothèse).**
Corriger ce site précis, et lui seul. Le correctif reste dans `_iterate_groom_loop` ; s'il déborde
sur `_arch_ask` / `_parse_*` / `_write_canonical_callout`, **arrêter et remonter** (Stop condition).

**Branche β — aucune occurrence post-2026-07-25 (l'hypothèse tient).**
Aucun changement de comportement. La sortie UNPARSED est déjà couverte par le ré-essai de mika#1823.
Le livrable devient la **preuve** : le test de la Phase 3 doit démontrer que, sans le ré-essai de
`f8b63530`, la boucle rend non-zéro sur la signature du 2026-07-04, et qu'avec lui elle converge.
C'est la seule chose que #1772 doit encore au dépôt dans cette branche.

### Phase 3 — Test de régression par fixture

5. Ajouter au bloc `mika#1772` existant de `test-dispatch-lib.sh` (à partir de la ligne 3524) une
   fixture qui **rejoue la signature du 2026-07-04** : première passe architecte répondant un contenu
   plausible **sans** ligne `Disposition:`, seconde passe convergeant `GROOMED`.
6. Le test doit **échouer si le mécanisme est retiré**. Le pas de vérification est explicite et
   obligatoire : neutraliser localement le ré-essai (branche β) ou le correctif (branche α), relancer,
   **constater l'échec**, restaurer. Un test qui passe dans les deux états ne prouve rien — c'est
   exactement le piège que le commentaire `A totals comparison was tried first and proved complaisant`
   (`test-dispatch-lib.sh:3574`) documente déjà sur ce même bloc.
7. Câbler la fixture dans la cible qui tourne en CI. La suite est déjà branchée dans `make test` et
   `.github/workflows/ci.yml` depuis `3f06baa3` — vérifier que le nouveau cas y est pris, ne pas
   recâbler.

### Phase 4 — Vérifier en vol

8. Faire converger **un** grooming réel de bout en bout : un ticket dont le plan est présent sur la
   branche, dispatché par le label `ready`, atteignant `_iterate_groom_loop`, et rendant `GROOMED`
   avec le callout canonique écrit sur le corps du ticket.
9. Consigner l'identifiant de tâche, le `session_id` architecte, et la ligne
   `converged on GROOMED for …`. C'est la mesure qui répond au critère (c) du ticket — un test vert
   ne le remplace pas.

**Obstacle connu, à traiter comme une étape et non comme une surprise :** au 2026-09-03, les dispatches
de grooming se refusent en `auto_skipped` / `already_groomed` sans jamais entrer dans la boucle. Le
ticket choisi pour AC4 doit donc être un ticket **non encore groomé**. S'il n'en existe aucun de
disponible, la vérification est bloquée sur un fait de substrat extérieur à #1772 : ne pas la
contourner, la remonter (Stop condition).

### Phase 5 — Compounder

10. Un document dans `docs/solutions/`, portant le seul enseignement qui se généralise :
    **une hypothèse d'investigation vieillit, et sa date la réfute avant toute mesure.** Deux des trois
    candidats de ce ticket étaient déjà dans le code au moment de l'incident qu'ils prétendaient
    expliquer ; sept semaines d'attente ont coûté plus cher que les deux `git log -S` qui les
    réfutent. Nommer la parade : dater les candidats contre l'historique du dépôt **avant** de les
    classer.

---

## Definition of Done

- **AC1.** La cause est classée dans l'une des quatre catégories du ticket (A / B / C / other), avec
  la sortie `return 1` nommée. La classification cite ses preuves ; là où la preuve d'origine n'existe
  plus, le plan le dit explicitement plutôt que de conclure sans elle.
  *Tie-back : critère de succès (a) du ticket.*
- **AC2.** L'absence de preuve historique est **établie par sonde**, pas supposée : les trois
  compteurs de la Phase 1 pas 1 sont consignés, contrôle positif inclus, et le résultat du pas 3
  (occurrence post-2026-07-25 trouvée ou non) est écrit.
  *Tie-back : rend le critère (a) honnête plutôt que non-tenu.*
- **AC3.** Un test de régression rejoue la signature du 2026-07-04 dans `test-dispatch-lib.sh`, et
  **il a été observé échouer** avec le mécanisme neutralisé. La preuve de cet échec figure dans le
  corps de la PR.
  *Tie-back : critère de succès (b) du ticket.*
- **AC4.** Un grooming réel a convergé `GROOMED` par `_iterate_groom_loop`, identifiant de tâche et
  `session_id` consignés. Si la convergence est empêchée par un fait de substrat extérieur à la
  boucle, l'AC est **remontée**, pas déclarée acquise.
  *Tie-back : critère de succès (c) du ticket.*
- **AC5.** Un document `docs/solutions/` porte l'enseignement de la Phase 5.
- **AC6.** Le corps du ticket est corrigé sur ses deux points périmés : la priorité (`p2-normal` →
  `p1-important`, per le commentaire du 2026-08-28T22:12Z) et la portée du fix (le candidat C est
  livré par PR#2028). Aucun contenu opérateur n'est supprimé — les commentaires restent la trace.

---

## Risques

- **R1 — La classification reste indécidable.** Si la Phase 1 ne trouve ni trace historique ni
  occurrence post-2026-07-25, AC1 se conclut sur « other : sortie non identifiable, preuve détruite »
  avec la table des 19 sorties comme livrable. C'est une réponse, pas un échec — et c'est
  l'aveu qu'il fallait faire il y a sept semaines. Le ticket ferme sur AC2–AC5.
- **R2 — AC4 reste bloquée par `auto_skipped`.** Traité en Phase 4 comme étape nommée. Ne pas
  neutraliser le garde `already_groomed` pour faire passer l'AC : ce serait affaiblir une protection
  pour satisfaire une mesure.
- **R3 — Le paradoxe du 2026-08-28 (« le groomer cassé ne peut pas groomer sa propre réparation »).**
  Il portait sur l'incapacité de `dev-groom` à commiter — cause mika#2141, corrigée le 2026-09-02.
  Le présent plan est d'ailleurs groomé hors boucle, par `/mika-groom-ticket` interactif ; le
  paradoxe n'est donc pas dans le chemin de ce ticket. À ne pas re-invoquer comme blocage sans
  nouvelle mesure.
- **R4 — Reclasser un défaut adjacent en défaut de convergence.** `idle_timeout`, `policy:deny`,
  gitdir absent : trois classes voisines déjà nommées par PR#2028 et mika#2141. La table de la
  Phase 1 pas 4 est ce qui empêche de les confondre.

---

## Registre de grooming

- **2026-09-03 — réconciliation AC ↔ plan (Phase 2.5) : zéro divergence bloquante.** Corps **et** les
  deux commentaires opérateur relus à froid. Trois supersessions relevées (priorité p2→p1, candidat C
  livré, item 1 diagnostiqué par mika#2141) : le ticket se corrige dans ses propres commentaires, ce
  n'est pas un désaccord corps↔plan. Rapport : `/tmp/groom-divergence-mika-1772.md`.
- **2026-09-03 — la branche `fix/1772/…` a été remise sur `origin/main`.** Elle portait encore les six
  commits pré-merge de PR#2028, tous absorbés par le squash `b84fdbc8` ; aucun fichier n'existait sur
  la branche qui ne soit sur `main` (`comm -23` sur les deux `ls-tree` : vide). Rien de perdu.
- **Incertitudes non arbitrées, portées à l'architecte.** (i) La branche β laisse #1772 sans
  changement de comportement — est-ce une fermeture légitime ou faut-il un ticket de suivi nommé ?
  (ii) AC6 fait modifier le corps du ticket par l'implémenteur : acceptable, ou geste opérateur ?
  (iii) Faut-il, au-delà de `tasks.result`, un puits durable pour le stderr de `dispatch-lib` — ou
  est-ce un ticket de substrat distinct ?
