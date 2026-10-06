---
module: mika-agent
tags: [source-scan, sole-reader, allowlist, live-pilot, anti-vacuity, premise]
problem_type: best_practice
---

# Un scan de lecteur unique se définit par les termes de la question, et sa population « vide » se mesure

**Mesuré :** 2026-10-06 (mika#2653, phase B, AC6).

---

## Le fait

Le plan groomé de mika#2653 (§ 9) prescrivait un scan refusant tout fichier de
production hors `live_pilot.rs` qui compose une traversée de dispatch
(`find_dispatch_children_with_pid`) avec `is_same_process_alive`, allowlist
**vide**, en affirmant que cette composition n'existait « nulle part
aujourd'hui ». La prémisse était fausse, et deux fois :

- `TaskEngine::dispatch_liveness` (`engine.rs`, mika#2156) compose exactement
  ces deux termes ;
- `process_kill.rs`, au niveau fichier, les composait aussi avant l'extraction.

Le scan livré tel quel aurait été rouge le jour de sa livraison, et la seule
façon de le passer vert avec une allowlist épinglée vide était d'exempter le
site — l'inverse de la doctrine.

## La résolution : le prédicat porte la question, pas deux mots-clés

La question que `live_pilot.rs` possède a **trois** termes : traversée, filtre
terminal (`is_terminal_task_status`), preuve d'instance. `dispatch_liveness`
refuse **délibérément** le deuxième (D-2 de mika#2156 : 1146 des 1147 enfants
porteurs de pid sont `delivered`, et rien ne prouve que `delivered` implique un
processus mort). C'est une **autre question**, écrite une fois et documentée au
site — pas le second lecteur silencieux que le scan existe pour refuser.

Le prédicat exige donc les trois termes. Allowlist vide, anti-vacuité sur le
propriétaire, et un contrôle de bonne foi qui montre le prédicat rouge sur la
forme **réelle** du `process_kill.rs` d'avant l'extraction (vu rouge aussi par
mutation sur l'arbre, pas seulement en fixture) et vert sur la forme réelle de
`dispatch_liveness`.

## Le coût, nommé : un lecteur sans le troisième terme échappe

La revue en a trouvé un second présent : `probe_pilot_liveness` (`mika-cli`,
`mika tasks show`). C'est la limite structurelle de cette forme. Un scan par
termes voit ceux qui posent **la même** question. Il ne voit pas ceux qui en
posent une voisine. Le résidu est donc écrit au test avec ses membres présents,
et non comme une éventualité future.

## Ce qui se généralise

1. **La prémisse « population mesurée vide » d'un plan se re-mesure contre
   l'arbre avant d'écrire le scan** : un `grep -l` des deux termes sur
   `crates/`. Un plan groomé a une demi-vie.
2. **Le prédicat d'un scan de lecteur unique énumère les termes qui définissent
   la question**, pas les deux appels les plus saillants. Le terme qui sépare la
   question d'une question voisine (ici le filtre terminal) est celui qui rend
   l'allowlist vide honnête.
3. **Le contrôle de bonne foi prend ses formes dans l'arbre** : le positif est
   la forme réelle que le scan aurait refusée, le négatif est la forme réelle
   qu'il doit laisser passer. Une fixture inventée prouve seulement que le
   prédicat matche ce que son auteur imaginait.
