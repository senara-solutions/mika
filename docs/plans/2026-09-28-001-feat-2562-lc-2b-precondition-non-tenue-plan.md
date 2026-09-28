# mika#2562 — LC.2b : la précondition n'est pas tenue, et une seconde s'y ajoute

> **Nature de ce plan : halte-et-remontée.** Il ne livre aucune ligne de code, et
> ce refus est son livrable. Le ticket porte une précondition explicite ; elle est
> mesurée **non tenue** sur ses deux voies, et le grooming en a trouvé une
> seconde que le corps ne nomme pas. Écrire ici une API pour `PasswordInput`,
> `AuthCard` et `Stepper` reproduirait mot pour mot le défaut que mika#1801 (LC.2)
> a refusé de commettre en reportant ces trois-là plutôt qu'en les devinant.

## 1. Ce que ce plan établit, et ce qu'il ne fait pas

Il établit **par mesure**, à l'instant du dispatch (`HEAD == origin/main`,
`65af9c8c`) :

1. La précondition déclarée du ticket n'est tenue par **aucune** de ses deux
   voies (§2).
2. Un **second** blocage, non déclaré dans le corps : le prédécesseur direct
   LC.2 / mika#1801 n'est pas mergé, et il modifie **exactement** les trois
   fichiers que ce ticket modifierait (§3).
3. Le corps du ticket contient une affirmation périmée — « LC.2 … a livré
   `<Button>` et `<Spinner>` » — qui n'est vraie que d'une branche, pas de `main`
   (§3).

Il ne propose **aucune signature, aucun nom de prop, aucun axe de discrimination**
pour les trois primitives. Ce silence est délibéré : voir §5.

## 2. La précondition déclarée, mesurée voie par voie

Le ticket la formule ainsi : *« Au moins un callsite réel lisible, ou une section
de rulebook décrivant le pattern. »* Une seule des deux voies suffirait.

### 2.1 Voie g1 — le rulebook décrit-il les patterns ? **Non.**

`docs/design/luminescent-core.md` a été lu section par section
(`grep -n '^#' docs/design/luminescent-core.md`, 20 titres). Les deux seules
sections qui pourraient passer pour une couverture n'en sont pas :

| section du rulebook | ce qu'elle décrit réellement | couvre-t-elle ? |
|---|---|---|
| `### Input Fields` (l. 185) | fond `surface_container_lowest`, bordure `outline_variant` à 10 %, état focus | **non** — un champ générique. Rien sur le masquage, la bascule reveal, l'`autocomplete`, ni aucune des décisions propres à un champ **secret** |
| `### Cards & Lists (The Divider-Free Approach)` (l. 109) | l'interdiction des filets de 1 px **entre éléments de liste**, remplacés par l'échelle d'espacement | **non** — une grammaire de liste, pas une carte de formulaire centrée |
| — | — | **`Stepper` : aucune section, aucune mention** |

La mesure est **à jour** : `git rev-list --count HEAD..origin/main` rend `0`, et le
dernier commit touchant le rulebook est `76a8b0de` du **2026-08-23**. Le rulebook
étant owned par Vincent et mis à jour par commits directs (non par PR, cf.
`CLAUDE.md` § `docs/design/`), une section ajoutée serait déjà sur `main`. Elle
n'y est pas.

§8 du rulebook réserve explicitement à Vincent l'ajout d'un pattern qu'il ne
décrit pas, avec une section nommant ses surfaces consommatrices (§8.3). Extraire
maintenant serait inventer trois grammaires en contournement de cette procédure.

### 2.2 Voie g2 — un callsite réel est-il lisible ? **Non.**

Deux mesures, et la seconde est structurelle.

**Population dans ce dépôt : zéro.**

```bash
grep -rniE 'type="password"|PasswordInput|SecretField' \
  --include='*.tsx' --include='*.ts' --include='*.jsx' --include='*.html' . \
  | grep -v node_modules            # → 0 ligne
grep -rniE 'AuthCard|CenteredCard|<Stepper' \
  --include='*.tsx' --include='*.ts' . | grep -v node_modules   # → 0 ligne
```

La seule occurrence du mot « stepper » dans l'arbre est de la prose :
`docs/brainstorms/2026-03-16-dashboard-tasks-teams-brainstorm.md:199`, qui décrit
*« un indicateur d'étape … comme un stepper »* pour un futur widget d'itérations.
Une intention de mars, pas un appelant.

**`mika-cloud` est absent du bac à sable, donc g2 est ici inexécutable.**

```bash
ls /data/workspace/mika-platform/     # → claude-pilot  mika
ls -d /data/workspace/mika-platform/mika-cloud
# → No such file or directory
```

C'est le point qui mérite d'être écrit plutôt que supposé. Le ticket ouvre g2
généreusement — *« même par simple lecture d'un fichier de la console. Un appelant
réel suffit à fixer l'API ; il n'a pas besoin de vivre dans ce dépôt pour être
lu »*. Cette générosité est correcte **et elle ne peut pas être exercée depuis une
session dispatchée sur `mika`** : le worktree ne matérialise que le sous-dépôt
`mika/`, et le répertoire parent ne porte que `claude-pilot` et `mika`. Le dépôt
`mika-cloud` n'est pas sur cet hôte à un chemin atteignable.

**Conséquence de conduite, et elle est le cœur de la remontée :** g2 ne peut être
tenue que par un geste qui **dépose le callsite dans ce dépôt ou dans le ticket** —
un extrait collé dans un commentaire d'issue, un fichier de fixture, ou le ticket
LC.3 portant le code. Un dispatch supplémentaire sur `mika` ne la tiendra jamais,
quel que soit le nombre de re-drives.

## 3. Le second blocage, non déclaré : LC.2 n'est pas mergé

Le corps affirme que LC.2 *« a livré `<Button>` et `<Spinner>` »*. La mesure dit
que c'est vrai **d'une branche**, pas de `main` :

```bash
ls packages/ui/src/components/        # 15 primitives, ni Button ni Spinner
git log --oneline --all -- packages/ui/src/components/Button.tsx
# → 307b04bd feat(ui,lc.2): extract <Button> and <Spinner> per rulebook §5 (mika#1801)
git rev-list --count origin/main..origin/feat/1801/ui-lc-2-extract-missing-primitives-dans
# → 13
```

`Button.tsx` n'existe que sur `feat/1801/ui-lc-2-extract-missing-primitives-dans`,
à **13 commits d'avance sur `main`**, dont le dernier date d'aujourd'hui
(`c94610a0`, 2026-09-28 — une réconciliation avec `main`). LC.2 est donc **en vol**,
et c'est cette même branche qui a créé ce ticket en report
(`2b466200 docs(plan,1801): transcrire l'AC1 ratifié (2 primitives, les 3 autres
vers mika#2562)`).

**Le recouvrement de fichiers est total, et c'est ce qui en fait un blocage et non
une gêne.** Les trois fichiers que le périmètre de mika#2562 modifierait sont
exactement ceux que mika#1801 modifie déjà :

```bash
git diff --name-only origin/main...origin/feat/1801/... \
  | grep -E 'packages/ui/(src/index.ts|CLAUDE.md|package.json)'
# → packages/ui/CLAUDE.md
#   packages/ui/package.json
#   packages/ui/src/index.ts
```

Trois conséquences concrètes :

- **`src/index.ts`** — les deux PR ajoutent des blocs d'export au même endroit :
  conflit garanti.
- **`CLAUDE.md`** — le point 4 du périmètre (« tableau des primitives mis à
  jour ») vise le tableau que LC.2 est en train de réécrire ; et LC.2 y ajoute
  déjà une règle d'enforcement CTA (`da009bae`, `905c17dc`).
- **`package.json`** — `main` est à `0.3.1` et LC.2 bumpe déjà. Un second bump
  mineur décidé sans voir le premier produit soit une collision de version, soit
  un saut silencieux.

**Ce blocage est indépendant de la précondition.** Même si Vincent commitait les
sections de rulebook dans l'heure, ce ticket resterait bloqué par l'ordre de
merge. C'est une seconde précondition, et le ticket ne la nomme pas.

## 4. Pourquoi ce ticket n'est pas simplement « à rétrécir »

La tentation, face à une précondition non tenue, est de livrer la part qui semble
sûre — typiquement `AuthCard` / `CenteredCard`, qui « n'est qu'une carte centrée ».
Elle est refusée, et la raison est mesurable plutôt que prudentielle :

1. **Une carte centrée sans formulaire à l'intérieur n'a pas d'API.** Ce qui fait
   la valeur d'un `AuthCard` est ce qu'il impose à son contenu — largeur maximale,
   place du logo, du titre, de l'erreur de formulaire, du lien secondaire. Chacune
   de ces décisions se lit sur un écran de login réel, et il n'y en a aucun ici.
2. **Le ticket a déjà été rétréci une fois, avec ratification.** `2b466200`
   transcrit l'AC1 ratifié de LC.2 : deux primitives livrées, trois reportées ici.
   Re-rétrécir ce report serait défaire un arbitrage opérateur déjà pris.
3. **Une règle d'enforcement sans population est une garde muette.** Le corps le
   dit déjà et la mesure le confirme : ajouter « hand-rolled Stepper is a review
   fail » à `packages/ui/CLAUDE.md` alors que l'arbre contient zéro `Stepper`
   produit une règle dont le silence se lit comme une conformité — la classe que
   ce dépôt a déjà dû nommer (mika#2205 : *une garde qu'on n'a pas déployée se lit
   exactement comme une flotte saine*).

## 5. Pourquoi ce plan ne propose aucune API, et pourquoi c'est le livrable

Un plan de grooming est normalement jugé sur la précision de son remède. Ici, la
précision serait le défaut. Trois primitives, trois surfaces d'invention :

- `PasswordInput` — la bascule reveal est-elle un bouton dans le champ ou à côté ?
  L'état révélé est-il contrôlé ou interne ? `autocomplete` vaut-il
  `current-password` ou `new-password` — et si les deux, l'axe est-il une prop ou
  deux composants ? Un écran de login et un écran d'invitation **ne répondent pas
  pareil**, et le ticket nomme les deux comme consommateurs.
- `AuthCard` — voir §4.1.
- `Stepper` — nombre d'étapes fixe ou dynamique ? Les étapes passées sont-elles
  cliquables ? Y a-t-il un état d'erreur par étape ? L'onboarding de la console
  décide, et il n'est pas lisible.

LC.2 a tranché ce genre de question avec une règle d'or que ce plan reprend
plutôt que de la contourner : sa décision D1 a discriminé **sur l'élément rendu**
(`<button>` vs `<a>`), pas sur la variante visuelle, *parce que les callsites
révélaient cet axe-là*. Le point 3 du périmètre de ce ticket la réénonce :
« union discriminée **là où elle porte de l'information**, sur l'axe que les
callsites révèlent — et non par principe ». Sans callsite, il n'y a pas d'axe à
révéler, et toute union serait décorative.

> Une API conçue sans appelant n'est pas une API imparfaite : c'est une API dont
> le premier appelant réel démontrera qu'elle est fausse, après qu'elle aura été
> publiée sous un numéro de version mineur et consommée.

## 6. Ce qui lève chaque blocage, et par qui

| # | blocage | geste qui le lève | propriétaire | atteignable depuis un dispatch `mika` ? |
|---|---|---|---|---|
| B1 | g1 : aucune section de rulebook | ajouter à `luminescent-core.md` les sections décrivant les trois patterns, en nommant leurs surfaces consommatrices (§8.3) | **Vincent** (le rulebook est mis à jour par commits directs) | non |
| B1′ | g2 : aucun callsite lisible | déposer au moins un appelant réel **dans ce dépôt ou dans le ticket** — extrait collé en commentaire d'issue, fixture, ou ticket LC.3 portant le code | LC.3 / `mika-cloud` | non |
| B2 | LC.2 non mergé, recouvrement de 3 fichiers | merger mika#1801 (13 commits d'avance, réconcilié avec `main` aujourd'hui) | boucle QA / opérateur | non |

B1 **ou** B1′ suffit pour la précondition déclarée. **B2 est cumulatif :** il doit
être levé en plus, et dans cet ordre — LC.2 d'abord, puisque c'est lui qui fixe la
forme du tableau, du bloc d'export et de la version que ce ticket étendrait.

**Aucun des trois gestes n'est exécutable par un pilote dispatché sur `mika`.**
C'est ce qui fait de ce plan une remontée et non une étape.

## 7. Le périmètre qui s'exécutera, quand les trois seront levés

Inchangé, et transcrit ici pour que le prochain dispatch n'ait pas à le rederiver.
Les points 1 à 5 sont ceux du corps du ticket ; la colonne de droite dit ce dont
chacun dépend.

| # | livrable | dépend de |
|---|---|---|
| 1 | les trois primitives dans `@samidarko/ui`, exportées depuis `src/index.ts` | B1/B1′ pour l'API, B2 pour la forme du bloc d'export |
| 2 | chacune : ≥ 1 test de comportement **et** une assertion `jest-axe` (`packages/ui/CLAUDE.md` § Accessibility Standards) | 1 |
| 3 | types stricts ; union discriminée seulement sur l'axe que les callsites révèlent | B1′ — c'est le callsite qui fournit l'axe |
| 4 | tableau des primitives de `packages/ui/CLAUDE.md` mis à jour ; règle d'enforcement **seulement** si la primitive a une population à garder | B2 (le tableau est en cours de réécriture par LC.2) |
| 5 | bump mineur de `@samidarko/ui` | B2 (`main` est à `0.3.1`, LC.2 bumpe déjà) |

Note sur le point 4, qui est le plus facile à mal exécuter : la clause
« seulement si la primitive a une population à garder » signifie que même après
extraction, **les trois primitives n'auront probablement encore aucune population
dans ce dépôt** — leurs consommateurs sont dans `mika-cloud`. Les six règles
d'enforcement existantes de `packages/ui/CLAUDE.md` (l. 34-41) sont toutes
formulées « Any **dashboard** … is a review fail », c'est-à-dire scopées sur une
population réellement présente. Une septième règle pour `Stepper` n'a de sens que
formulée sur la surface qui le consomme, donc dans le dépôt qui la contient.

## 8. Le gate détecteur de mika#2306 ne s'applique pas

Ce plan ne livre **aucun** détecteur : ni test, ni assertion, ni règle de lint, ni
garde CI, ni validateur de schéma, ni scan structurel, ni garde EndTurn. Il ne
livre aucune ligne de code. La section `## Fire-Disposition` n'est donc pas
requise — gate **N/A** au sens littéral de la règle mika#2306, qui prescrit de ne
pas l'inventer quand il n'y a pas de détecteur à disposer.

Cette absence est écrite plutôt que laissée au silence, parce qu'un titre manquant
et un titre non requis se lisent identiquement.

Pour mémoire et sans l'appliquer à un détecteur : la disposition du **plan** est
l'option (c) de mika#1574, halte-et-remontée — l'implémentation s'arrête et remonte
à l'opérateur pour cadrage. C'est la disposition que le corps du ticket prescrit
déjà de lui-même (*« Dispatcher ce ticket avant que l'un des deux soit tenu
reproduit exactement le défaut que LC.2 a refusé de commettre »*).

## 9. Vérification

Ce plan n'introduisant aucun code, sa vérification est la reproductibilité de ses
mesures. Chacune est une commande, et chacune doit rendre le même résultat qu'en
§2 et §3 tant que les blocages tiennent :

```bash
# B1 — aucune des trois sections de rulebook
grep -niE 'password|secret field|auth card|centered card|stepper' \
  docs/design/luminescent-core.md            # attendu : 0 ligne

# B1′ — aucun callsite, et mika-cloud hors d'atteinte
grep -rniE 'type="password"|PasswordInput|SecretField|AuthCard|CenteredCard|<Stepper' \
  --include='*.tsx' --include='*.ts' . | grep -v node_modules   # attendu : 0 ligne
ls -d /data/workspace/mika-platform/mika-cloud   # attendu : No such file or directory

# B2 — LC.2 en vol, recouvrement des trois fichiers
git rev-list --count origin/main..origin/feat/1801/ui-lc-2-extract-missing-primitives-dans
git diff --name-only origin/main...origin/feat/1801/ui-lc-2-extract-missing-primitives-dans \
  | grep -E 'packages/ui/(src/index.ts|CLAUDE.md|package.json)'   # attendu : les 3
```

**Trois haltes à la relecture de ce plan :**

- **Halte 1 — les commentaires du ticket ne m'ont pas été livrés.** `gh` n'est pas
  authentifié dans ce bac à sable (`gh auth login` requis), et le contexte injecté
  ne portait que le corps. Si un commentaire opérateur tient g1 ou g2 — par exemple
  un extrait de callsite collé par Vincent, ce que g2 autorise explicitement — alors
  §2.2 est **périmé** et ce plan doit être révisé plutôt que suivi. C'est la
  première chose à établir avant d'agir sur cette remontée.
- **Halte 2 — si une mesure de §9 cesse de rendre le résultat attendu**, le blocage
  correspondant est levé : ne pas re-dispatcher à l'aveugle, relire §6 pour savoir
  s'il restait cumulatif avec un autre.
- **Halte 3 — ne pas lever B2 en rebasant ce ticket sur `feat/1801`.** La tentation
  est réelle (elle fournirait `Button`, le tableau et la version), et elle
  empilerait ce travail sur une branche non revue de 13 commits : un `hold[review]`
  sur LC.2 emporterait les deux. L'ordre est `main` ← LC.2, puis LC.2b.

## 10. Definition of Done

- [x] La précondition du ticket est mesurée voie par voie, avec les commandes qui
      la reproduisent (§2).
- [x] L'inexécutabilité structurelle de g2 depuis un dispatch `mika` est établie
      et écrite, avec sa conséquence de conduite (§2.2).
- [x] Le second blocage — LC.2 non mergé, recouvrement de trois fichiers — est
      mesuré et nommé (§3).
- [x] L'affirmation périmée du corps (« LC.2 a livré `<Button>` et `<Spinner>` »)
      est corrigée par la mesure (§3).
- [x] Aucune API n'est proposée pour les trois primitives, et le refus est motivé
      primitive par primitive (§5).
- [x] Le geste qui lève chaque blocage est nommé avec son propriétaire, et
      l'ordre cumulatif est dit (§6).
- [x] Le périmètre d'exécution futur est transcrit, chaque point annoté de sa
      dépendance (§7).
- [x] La non-applicabilité du gate mika#2306 est déclarée avec sa raison (§8).
- [x] Les trois haltes de relecture sont écrites, dont la limite de ma propre
      mesure (§9).
- [ ] **Non fait, et c'est le contrat :** aucun code, aucun test, aucun export,
      aucun bump de version, aucune règle d'enforcement.

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria`. Les critères ci-dessous
sont dérivés de sa précondition et du périmètre de ce plan de halte. **Ils portent
sur la remontée, pas sur l'extraction** — les critères de l'extraction elle-même
sont ceux du §7, et ils ne deviennent évaluables qu'une fois B1/B1′ et B2 levés.

1. **AC1 — la précondition est tranchée par mesure, pas par citation.** Les deux
   voies g1 et g2 sont chacune évaluées par une commande reproductible dont le
   résultat est donné, et la mesure est datée de l'instant du dispatch
   (`HEAD == origin/main`, aucun retard).
2. **AC2 — l'inexécutabilité de g2 depuis `mika` est établie.** Le plan démontre
   que `mika-cloud` n'est pas atteignable depuis le bac à sable de dispatch, et en
   tire la conséquence : aucun re-drive sur `mika` ne peut tenir g2, seul un dépôt
   du callsite dans ce dépôt ou dans le ticket peut le faire.
3. **AC3 — le second blocage est nommé.** Le plan établit que mika#1801 n'est pas
   mergé, que son recouvrement avec le périmètre de ce ticket porte sur les trois
   fichiers `packages/ui/src/index.ts`, `packages/ui/CLAUDE.md` et
   `packages/ui/package.json`, et que ce blocage est cumulatif avec la précondition
   plutôt qu'alternatif.
4. **AC4 — aucune API n'est inventée.** Le plan ne contient aucune signature,
   aucun nom de prop, aucun axe de discrimination, aucun squelette de composant
   pour `PasswordInput` / `SecretField`, `AuthCard` / `CenteredCard`, `Stepper`.
   Le refus est motivé pour chacune des trois, en citant l'axe qu'un callsite
   aurait révélé.
5. **AC5 — aucune règle d'enforcement sans population.** Aucune ligne n'est
   ajoutée à `packages/ui/CLAUDE.md`, et le plan dit pourquoi une telle règle
   resterait muette même après extraction (les consommateurs vivent dans
   `mika-cloud`).
6. **AC6 — la remontée est actionnable.** Chaque blocage porte le geste qui le
   lève, son propriétaire, et la mention explicite qu'aucun des trois n'est
   exécutable par un pilote dispatché sur `mika`.
7. **AC7 — l'arbre est inchangé hors de ce fichier.** Le diff de cette branche ne
   contient que ce plan : aucun fichier sous `packages/ui/`, `dashboard/`,
   `site/` ou `crates/` n'est touché.
8. **AC8 — la limite de la mesure est écrite.** Le plan déclare que les
   commentaires du ticket ne lui ont pas été livrés (`gh` non authentifié) et que
   la lecture d'un commentaire tenant g1 ou g2 périme §2.2.

## 11. Hors périmètre, délibérément

- **L'extraction des trois primitives.** C'est le périmètre du ticket, et il
  s'ouvre quand §6 est tenu. Ce plan ne le réduit pas : il en date la
  précondition.
- **L'adoption customer-facing** — c'est LC.3, comme le corps le dit déjà.
- **Une quatrième variante de `<Button>`** (`confirm` / `destructive`) — décision
  produit §8, ticket séparé si Vincent la veut (LC.2 D4).
- **Merger mika#1801, ou le rebaser sur ce ticket.** Le merge appartient à la
  boucle QA et à l'opérateur ; l'empilement est explicitement refusé (§9, halte 3).
- **Ajouter au rulebook les sections manquantes.** §8 du rulebook les réserve à
  Vincent. Les écrire ici serait le contournement de procédure que §2.1 nomme, et
  la voie g1 cesserait d'être une levée de précondition pour devenir une invention
  de plus, signée d'une autre main.
- **Fermer ou re-scoper mika#2562.** Ce plan remonte une précondition non tenue ;
  décider si le ticket attend, se scinde ou change de dépôt est un cadrage
  opérateur, et c'est précisément ce que la disposition halte-et-remontée demande.
