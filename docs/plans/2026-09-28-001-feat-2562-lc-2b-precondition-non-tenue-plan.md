# mika#2562 — LC.2b : la halte est levée, l'extraction est livrée

> **Nature de ce plan : halte-et-remontée, puis son exécution.** Il a été écrit
> comme un refus — la précondition du ticket était mesurée non tenue sur ses deux
> voies, et le grooming en avait trouvé une seconde que le corps ne nomme pas.
> **Les trois blocages sont tombés**, le dernier par le geste que ce plan appelait
> nommément, et le périmètre de §7 a été exécuté.
>
> **Ordre de lecture.** §0 pour l'état des trois blocages et l'instant de chaque
> levée. **§5.1 pour ce que les appelants mesurés fixent, axe par axe** — c'est ce
> qui remplace le refus d'API de §5, et c'est la section à lire avant le code.
> Les mesures antérieures sont conservées avec leur date plutôt qu'effacées : un
> plan qui réécrit ses mesures pour n'afficher que l'état courant cesse d'être une
> mesure pour devenir une opinion.
>
> **Le nom du fichier dit encore `precondition-non-tenue`, et il n'est pas
> renommé.** Le callout `> - **Plan:**` du corps du ticket pointe ce chemin
> littéral, et c'est lui que lisent `auto_pull::is_groomed` et
> `dispatch-lib.sh::_extract_plan_path` ; renommer sans pouvoir éditer le corps
> (`gh` non authentifié dans ce bac à sable) casserait le routage groom/implement
> au lieu de clarifier quoi que ce soit. Le titre ci-dessus porte l'état réel.

## 0. État des trois blocages — les trois sont tombés

Ce plan a été groomé, puis re-mesuré à deux dispatches successifs qui ont
maintenu sa halte, puis **levé**. Le tableau porte les trois verdicts avec
l'instant de chacun ; les commandes qui les fondent sont celles de §9.

| # | blocage | verdict au grooming (§2, §3) | verdict courant | ce qui l'a levé |
|---|---|---|---|---|
| B1 | g1 : aucune section de rulebook | non tenu | **toujours non tenu** — et sans objet | rien. Le rulebook est resté à `76a8b0de` (2026-08-23). La précondition porte sur g1 **ou** g2 ; g2 étant tenue, B1 ne bloque plus (§6) |
| B1′ | g2 : aucun callsite lisible | non tenu | **LEVÉ** | commentaire opérateur sur mika#2562, 2026-09-28T16:04:52Z : trois appelants réels lus dans `mika-cloud` à `origin/main` 5cb2544 |
| B2 | LC.2 non mergé, recouvrement de 3 fichiers | bloquant | **LEVÉ** | `fab69e13` (PR #2556) mergé sur `origin/main` ; `packages/ui` à `0.4.0`, 17 primitives, `Button.tsx` et `Spinner.tsx` présents |

**La levée de B1′ est exactement le geste que ce plan appelait**, et par la voie
qu'il désignait comme la moins chère (§6 : *« Le geste le moins cher est B1′, et
il tient en une ligne de commentaire »*). Le pilote précédent avait écrit
« `mika-cloud` absent de l'hôte » : vrai de son **bac à sable**, qui ne monte pas
ce dépôt, et faux de l'hôte — c'est la limite que §2.2 avait nommée comme
structurelle, et elle l'était bien, puisque seul un geste extérieur au dispatch
l'a franchie.

**Conséquence de conduite, et c'est un renversement, pas une nuance :** la
précondition déclarée du ticket porte sur g1 **ou** g2. g2 est tenue, donc
**la précondition est tenue**, B1 cesse d'être nécessaire, et le périmètre de §7
s'ouvre. La disposition halte-et-remontée est **éteinte** ; ce plan n'est plus un
refus mais le compte rendu de la mesure qui l'a précédé, plus les axes que les
appelants fixent (§5.1).

*Ancrage, remplaçable et non cumulatif : un dispatch ultérieur remplace ce
paragraphe par sa propre mesure, il ne l'empile pas.* Les mesures datées des
sections suivantes sont conservées telles quelles.

**Ce que ce plan n'a toujours pas pu établir lui-même.** `gh` n'est authentifié
dans aucun des trois bacs à sable successifs (`gh auth status` → *not logged into
any GitHub hosts*). La Halte 1 de §9 avait tiré la bonne conséquence — *aucun
re-dispatch ne la lèvera, quel qu'en soit le nombre* — et c'est en effet un geste
d'opérateur qui l'a levée, en injectant le contenu des commentaires dans le
contexte de dispatch. La halte est donc **réalisée**, pas contournée.

## 1. Ce que ce plan établit, et ce qu'il ne fait pas

Il établit **par mesure**, à l'instant du grooming (`HEAD == origin/main`,
`65af9c8c`) puis à celui du dispatch d'exécution (`fab69e13`, §0) :

1. La précondition déclarée du ticket n'est tenue par **aucune** de ses deux
   voies (§2). **Vrai aux deux instants** — c'est ce qui maintient la halte.
2. Un **second** blocage, non déclaré dans le corps : le prédécesseur direct
   LC.2 / mika#1801 n'est pas mergé, et il modifie **exactement** les trois
   fichiers que ce ticket modifierait (§3). **Vrai au grooming, levé depuis**
   (§0).
3. Le corps du ticket contient une affirmation périmée — « LC.2 … a livré
   `<Button>` et `<Spinner>` » — qui n'est vraie que d'une branche, pas de `main`
   (§3). **Vraie au grooming, et le merge de LC.2 l'a rendue exacte depuis** : le
   corps du ticket n'a plus besoin d'être corrigé sur ce point.

Il n'a proposé, tant que la précondition n'était pas tenue, **aucune signature,
aucun nom de prop, aucun axe de discrimination** pour les trois primitives. Ce
silence était délibéré (§5) et il a duré exactement aussi longtemps qu'il
devait : §5.1 le remplace, et chacun de ses axes cite l'appelant qui le fixe.

## 2. La précondition déclarée, mesurée voie par voie

Le ticket la formule ainsi : *« Au moins un callsite réel lisible, ou une section
de rulebook décrivant le pattern. »* Une seule des deux voies suffirait.

### 2.1 Voie g1 — le rulebook décrit-il les patterns ? **Non.**

`docs/design/luminescent-core.md` a été lu section par section
(`grep -n '^#' docs/design/luminescent-core.md` — **27 titres** ; le grooming
annonçait 20, comptage corrigé à la re-mesure sur un fichier inchangé, et ce
nombre n'est décisionnel pour rien : ce qui tranche est le `grep` des cinq mots,
qui rend 0 ligne aux deux instants). Les deux seules sections qui pourraient
passer pour une couverture n'en sont pas :

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

### 2.2 Voie g2 — un callsite réel est-il lisible ? **Non au grooming — LEVÉ depuis**

> **Statut : levé le 2026-09-28T16:04:52Z**, par un commentaire opérateur portant
> trois appelants réels lus dans `mika-cloud` à `origin/main` 5cb2544 — §0 pour la
> levée, §5.1 pour ce qu'ils fixent. Cette section est conservée plutôt
> qu'effacée parce que sa seconde mesure — l'inexécutabilité de g2 **depuis un
> dispatch** — est restée vraie de bout en bout et explique pourquoi la levée ne
> pouvait venir que du dehors.

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

## 3. Le second blocage, non déclaré : LC.2 n'était pas mergé — **levé**

> **Statut : résolu au dispatch d'exécution** (§0). Cette section est conservée
> plutôt qu'effacée pour deux raisons : elle établit *pourquoi* l'ordre de merge
> était contraignant, et cette raison gouverne encore la forme qu'aura le futur
> périmètre (§7) ; et un plan qui réécrit ses mesures pour n'afficher que l'état
> courant cesse d'être une mesure. La levée et ce qu'elle fixe sont en §3.3.

### 3.1 Ce qui était mesuré au grooming (`65af9c8c`)

Le corps affirme que LC.2 *« a livré `<Button>` et `<Spinner>` »*. La mesure disait
alors que c'était vrai **d'une branche**, pas de `main` :

```bash
ls packages/ui/src/components/        # 15 primitives, ni Button ni Spinner
git log --oneline --all -- packages/ui/src/components/Button.tsx
# → 307b04bd feat(ui,lc.2): extract <Button> and <Spinner> per rulebook §5 (mika#1801)
git rev-list --count origin/main..origin/feat/1801/ui-lc-2-extract-missing-primitives-dans
# → 13
```

`Button.tsx` n'existait alors que sur
`feat/1801/ui-lc-2-extract-missing-primitives-dans`, à **13 commits d'avance sur
`main`**, dont le dernier datait du jour même (`c94610a0`, 2026-09-28 — une
réconciliation avec `main`). LC.2 était donc **en vol**,
et c'est cette même branche qui a créé ce ticket en report
(`2b466200 docs(plan,1801): transcrire l'AC1 ratifié (2 primitives, les 3 autres
vers mika#2562)`).

### 3.2 Le recouvrement de fichiers, et pourquoi il faisait un blocage

**Le recouvrement était total, et c'est ce qui en faisait un blocage et non une
gêne.** Les trois fichiers que le périmètre de mika#2562 modifierait sont
exactement ceux que mika#1801 modifiait déjà :

```bash
git diff --name-only origin/main...origin/feat/1801/... \
  | grep -E 'packages/ui/(src/index.ts|CLAUDE.md|package.json)'
# → packages/ui/CLAUDE.md
#   packages/ui/package.json
#   packages/ui/src/index.ts
```

Trois conséquences concrètes, toutes au conditionnel d'alors :

- **`src/index.ts`** — les deux PR auraient ajouté des blocs d'export au même
  endroit : conflit garanti.
- **`CLAUDE.md`** — le point 4 du périmètre (« tableau des primitives mis à
  jour ») visait le tableau que LC.2 était en train de réécrire ; et LC.2 y
  ajoutait déjà une règle d'enforcement CTA (`da009bae`, `905c17dc`).
- **`package.json`** — `main` était à `0.3.1` et LC.2 bumpait déjà. Un second
  bump mineur décidé sans voir le premier aurait produit soit une collision de
  version, soit un saut silencieux.

**Ce blocage était indépendant de la précondition.** Même si Vincent avait
committé les sections de rulebook dans l'heure, ce ticket serait resté bloqué par
l'ordre de merge. C'était une seconde précondition, et le ticket ne la nomme pas.

### 3.3 La levée, et ce qu'elle fixe pour la suite

`fab69e13` (*« feat(ui,lc.2) : extraire Button et Spinner dans `@samidarko/ui`,
migrer les CTA, armer le détecteur »*, PR #2556) est mergé et **est le HEAD de
`origin/main`**. L'ordre prescrit en §6 — `main` ← LC.2, puis LC.2b — est donc
respecté sans qu'aucun geste ne reste à poser de ce côté.

Ce que le merge **fixe**, et qui n'avait pas à être deviné :

| axe | état fixé sur `main` |
|---|---|
| bloc d'export de `src/index.ts` | écrit par LC.2 ; un ajout futur s'y insère au lieu d'entrer en conflit |
| tableau des primitives de `packages/ui/CLAUDE.md` | réécrit par LC.2 ; **17** primitives, `Button.tsx` et `Spinner.tsx` compris |
| règles d'enforcement | **7**, toutes scopées sur une population présente (`dashboard`, `dashboard/src`, `site/src`) ; la septième porte une **moitié machine**, `scripts/check-cta-primitives.sh` |
| version de `@samidarko/ui` | `0.4.0` — le point 5 du périmètre futur part de là, pas de `0.3.1` |

Trois conséquences de lecture :

- **La halte 3 de §9 devient sans objet.** Elle interdisait de lever B2 en
  rebasant ce ticket sur `feat/1801` ; cette branche est mergée, il n'y a plus
  rien sur quoi empiler.
- **L'affirmation du corps du ticket est devenue exacte.** LC.2 *a* livré
  `<Button>` et `<Spinner>` — sur `main`, désormais. Le point 3 de §1 n'appelle
  plus de correction du corps.
- **Rien de tout cela ne touche la précondition.** B2 était cumulatif, jamais
  alternatif : le lever ne fournit ni section de rulebook, ni callsite.

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

## 5.1 Ce que les appelants fixent — les axes, et qui les tranche

Les trois appelants ont été lus dans `mika-cloud` à `origin/main` 5cb2544 et
déposés dans le ticket (commentaire du 2026-09-28T16:04:52Z). Chaque ligne
ci-dessous reprend **une question que §5 posait** et donne la réponse de
l'appelant. Aucune n'est tranchée par principe.

### `SecretField` — `web/src/pages/onboarding/ApiKeysStep.tsx`

| question de §5 | réponse mesurée | conséquence d'API |
|---|---|---|
| `autocomplete` vaut-il `current-password` ou `new-password` — et si les deux, l'axe est-il une prop ou deux composants ? | **Ni l'un ni l'autre.** Le seul champ masqué de la console est une **clé API** ; l'authentification passe entièrement par Google OAuth et aucun écran ne porte de mot de passe. La valeur mesurée est `off`. | **Aucune union.** `autoComplete` est une prop plate de défaut `off`. L'axe que §5 anticipait n'existe pas — le discriminer aurait produit l'union décorative que D1 de LC.2 refuse |
| la bascule reveal est-elle un bouton dans le champ ou à côté ? | **Dans le champ**, `absolute right-2 top-1/2`, avec `pr-14` sur l'input pour lui faire place | `.mika-field-wrap` en `position: relative`, contrôle en absolu. `pr-14` (3,5 rem) est **hors de l'échelle §6** : recalé sur `--spacing-16` |
| l'état révélé est-il contrôlé ou interne ? | **Interne** (`showKey` en état local, rien à l'extérieur ne le lit) | `useState` dans la primitive. Pas de prop, donc pas d'axe contrôlé/non-contrôlé |
| — (non posée par §5, imposée par l'appelant) | l'erreur de champ est rendue **sous** l'input **par l'appelant** | la primitive **ne rend pas** l'erreur ; elle expose `invalid` + `describedBy`, sans quoi l'extraction rendrait cette erreur inatteignable par un lecteur d'écran |
| — | le contrôle est un bouton **texte** (`Show` / `Hide`), pas une icône | rendu par `<Button variant="tertiary" size="sm">` — §5 définit tertiary comme « text-only using `primary` color … for low-priority actions », soit exactement ce contrôle |

**Le nom est une décision mesurée.** Le ticket offre `PasswordInput` / `SecretField`
sans trancher. L'appelant tranche : il n'existe aucun mot de passe dans la
console, donc `PasswordInput` nommerait une population de zéro. C'est
`SecretField`.

### `AuthCard` — `web/src/pages/Login.tsx` et `web/src/pages/Signup.tsx`

§4.1 posait la question ainsi : *« ce qui fait la valeur d'un `AuthCard` est ce
qu'il impose à son contenu — largeur maximale, place du logo, du titre, de
l'erreur de formulaire, du lien secondaire »*. Les deux appelants répondent, et
ils répondent **identiquement**, ce qui est la condition pour que la forme soit
une primitive et non la page d'un seul écran.

| axe | réponse mesurée | conséquence d'API |
|---|---|---|
| largeur maximale | `w-full max-w-sm` | `max-width: 24rem` dans la primitive |
| la primitive porte-t-elle le centrage plein écran ? | **oui** — `flex min-h-screen items-center justify-center`, mot pour mot dans les deux | `.mika-auth-screen` en fait partie. C'est la moitié « centrée » du couple `AuthCard` / `CenteredCard` |
| place du logo | en-tête centré, au-dessus du titre | slot `logo?: ReactNode`, **injecté par le consommateur** — la bibliothèque ne peut pas dépendre du `<Logo />` de la console (même séparation que `<AgentFilter agents>`) |
| titre, sous-titre | `h1` puis un sous-titre muted, centrés | `title` / `subtitle?`. Niveau `h1` **non configurable** : sur les deux appelants la carte *est* la page |
| erreur de formulaire, lien secondaire | **rien de commun** — un CTA pleine largeur et un lien d'alternance, différents entre les deux | `children` libre. Aucune API n'est tirée d'une forme qui varie |
| le pied de page ? | **hors de la carte**, fixé en bas, et sur `Login` **seulement** | **exclu de la primitive.** Une prop `footer` aurait été une API pour un appelant sur deux |

**Le nom retenu est `AuthCard`** : la grammaire d'en-tête qu'il impose est
propre à l'authentification, et le nommer par sa forme (`CenteredCard`)
l'inviterait sur des surfaces qui veulent la boîte sans la grammaire.

### `Stepper` — `web/src/pages/Onboarding.tsx`, fonction `StepProgress`

| question de §5 | réponse mesurée | conséquence d'API |
|---|---|---|
| nombre d'étapes fixe ou dynamique ? | **dynamique**, fonction du tier : `getStepIds(tier)` rend 3 ou 4 identifiants | `steps: { id, label }[]` en prop |
| les étapes passées sont-elles cliquables ? | **non** — l'étape est pilotée par l'état serveur via `statusToStep` | aucune prop `onStepClick`, et surtout : un `<ol>` et **non** un `<nav>`. Un point de repère de navigation qui ne contient rien de navigable est un mensonge fait aux technologies d'assistance |
| y a-t-il un état d'erreur par étape ? | **non**, l'appelant n'en a aucun | trois états seulement : `complete` / `current` / `upcoming` |
| — (non posée par §5, et décisive) | l'étape terminale `success` est **exclue de l'affichage** par l'appelant, alors que `statusToStep` peut la rendre comme courante | `current` peut nommer une étape **absente** de `steps`. Lu comme « au-delà de la fin » : toutes les étapes visibles sont complètes — ce qui est vrai à cet instant-là. Coût nommé : une faute de frappe dans `current` rend la même chose |
| `current` : identifiant ou index ? | la liste dépend du tier, donc l'index 1 est `apikey` sur un tier et `provisioning` sur un autre | **identifiant.** Un index désignerait silencieusement une autre étape selon le locataire |
| le connecteur | `w-8 h-px`, accent/50 si l'étape derrière est faite, neutre sinon | `.mika-stepper__connector`. §5 interdit les filets d'1 px **entre éléments de liste** et §7 les bordures prises « pour résoudre un problème de mise en page » : un connecteur de progression n'est ni l'un ni l'autre — il porte un état. Nommé plutôt que fait en silence |

### Ce que les appelants ne fixent pas, et qui n'a donc pas été écrit

`disabled`, `required`, `name` sur `SecretField` : aucun appelant ne les passe.
Les ajouter « par symétrie » aurait rouvert des sous-décisions que rien ne
tranche — notamment si la bascule reveal reste vivante sur un champ désactivé,
qui est une question de produit et non de primitive. Elles sont additives le jour
où un appelant les demande.

## 6. Ce qui lève chaque blocage, et par qui

| # | blocage | statut | geste qui le lève | propriétaire | atteignable depuis un dispatch `mika` ? |
|---|---|---|---|---|---|
| B1 | g1 : aucune section de rulebook | **ouvert, et sans objet** | ajouter à `luminescent-core.md` les sections décrivant les trois patterns, en nommant leurs surfaces consommatrices (§8.3) | **Vincent** (le rulebook est mis à jour par commits directs) | non |
| B1′ | g2 : aucun callsite lisible | ~~ouvert~~ **LEVÉ** (2026-09-28T16:04:52Z) | déposer au moins un appelant réel **dans ce dépôt ou dans le ticket** | opérateur, par commentaire d'issue | non — et c'est bien du dehors qu'il est venu |
| B2 | LC.2 non mergé, recouvrement de 3 fichiers | ~~bloquant~~ **LEVÉ** (`fab69e13`, PR #2556) | — | boucle QA / opérateur | — |

B1 **ou** B1′ suffit pour la précondition déclarée. **B1′ est tenu**, donc la
précondition l'est, et B1 n'est plus nécessaire. B2 était cumulatif — il devait
être levé *en plus*, et dans l'ordre `main` ← LC.2 puisque c'est lui qui fixait
la forme du tableau, du bloc d'export et de la version ; **il l'a été** (§3.3).
Les trois conditions sont donc satisfaites, chacune par son propriétaire.

**B1 reste ouvert, et ce n'est pas un reliquat à refermer en passant.** Le
rulebook ne décrit toujours ni la grammaire du champ secret, ni la carte
d'authentification, ni le stepper. §8 réserve à Vincent l'ajout d'un pattern
qu'il ne décrit pas ; les implémentations livrées sont donc dérivées des
appelants mesurés **plus** les contraintes §6/§7 qui lient toute surface, jamais
d'une section inventée. Le manque est remonté comme suivi opérateur dans
`packages/ui/CLAUDE.md`, exactement comme LC.1 a remonté la contradiction du hex
d'erreur §5.5/§2 au lieu de la trancher dans le rulebook.

**Ce qui a levé B1′ est le geste que cette section désignait comme le moins
cher**, et il a effectivement tenu en une ligne de commentaire : le ticket
l'autorise explicitement (*« même par simple lecture d'un fichier de la
console »*). Il a fallu qu'il vienne d'un opérateur — §2.2 avait établi que
c'était structurel, et trois bacs à sable successifs l'ont confirmé.

## 7. Le périmètre, et ce qui a été livré

Les points 1 à 5 sont ceux du corps du ticket.

| # | livrable | état | ce qui a été fait |
|---|---|---|---|
| 1 | les trois primitives dans `@samidarko/ui`, exportées depuis `src/index.ts` | **livré** | `SecretField.tsx`, `AuthCard.tsx`, `Stepper.tsx` + présentation dans `theme.css` ; exports ajoutés au bloc que LC.2 a fixé |
| 2 | chacune : ≥ 1 test de comportement **et** une assertion `jest-axe` | **livré** | 49 tests sur les trois, dont 8 assertions `axe` couvrant chaque état rendu |
| 3 | types stricts ; union discriminée seulement sur l'axe que les callsites révèlent | **livré** | **aucune union** sur les trois, et c'est le résultat de la mesure : §5.1 montre que chaque axe candidat est tranché identiquement par l'appelant unique. Une union y aurait été une interface sous plusieurs noms |
| 4 | tableau des primitives mis à jour ; règle d'enforcement **seulement** si la primitive a une population à garder | **livré** | 3 lignes ajoutées au tableau ; **zéro règle d'enforcement**, avec le motif écrit — voir la note ci-dessous |
| 5 | bump mineur de `@samidarko/ui` | **livré** | `0.4.0` → `0.5.0` |

**Sur la présentation, un point que le périmètre ne nommait pas et qui décide de
tout.** LC.2 a mesuré que **Tailwind ne scanne pas `packages/ui`** : une
utilitaire écrite dans ce paquet n'est jamais générée. Les trois primitives
auraient donc été livrées sans surface, sans rayon et sans anneau de focus si
elles avaient été bâties sur des utilitaires. Leur présentation vit dans
`theme.css`, comme celle de `<Button>`, et pour la même raison mesurée — elle
compte davantage ici, le seul consommateur des trois étant `mika-cloud`, qui
importe `theme.css` et rien d'autre de la configuration de build de ce paquet.
Contrôle positif et négatif exécutés après coup sur le CSS bâti du dashboard :
les nouvelles règles `.mika-*` y sont, et `.w-9` (écrite uniquement dans
`packages/ui`) n'y est toujours pas.

Note sur le point 4, qui est le plus facile à mal exécuter : la clause
« seulement si la primitive a une population à garder » signifie que même après
extraction, **les trois primitives n'auront encore aucune population dans ce
dépôt** — leurs consommateurs sont dans `mika-cloud`, et la re-mesure de §0 le
confirme à la date du dispatch d'exécution. Les **sept** règles d'enforcement de
`packages/ui/CLAUDE.md` sont toutes formulées « Any **dashboard** … is a review
fail » ou « under `dashboard/src` or `site/src` », c'est-à-dire scopées sur une
population réellement présente. Une huitième règle pour `Stepper` n'a de sens que
formulée sur la surface qui le consomme, donc dans le dépôt qui la contient.

**Le merge de LC.2 durcit cet argument au lieu de l'affaiblir.** Sa septième règle
est la première à porter une **moitié machine** — `scripts/check-cta-primitives.sh`,
job CI `cta-primitives-lint` — et son allowlist est livrée **vide et pinnée vide**.
Transposer ce motif à `Stepper` produirait un scan dont la population est nulle par
construction : il passerait au vert tous les jours sans rien regarder, et son
silence se lirait comme une conformité. C'est nommément la classe mika#2205 — *une
garde qu'on n'a pas déployée se lit exactement comme une flotte saine* — et la
raison pour laquelle le point 4 dit « seulement si ».

## 8. Fire-Disposition

> **Ce titre n'était pas requis tant que ce plan était une halte** : il ne livrait
> alors aucun détecteur, et mika#2306 prescrit de ne pas inventer la section quand
> il n'y a rien à disposer. La levée de B1′ change cela — l'extraction livre des
> détecteurs, et voici leur disposition.

Les détecteurs livrés sont des tests unitaires, exécutés par
`npm test --prefix packages/ui` et par la CI. Aucune garde CI nouvelle, aucun scan
de source, aucun script — et c'est délibéré : voir la note sur la population nulle
au point 4 de §7.

| détecteur | ce qu'il refuse | disposition quand il tire |
|---|---|---|
| `SecretField.test.tsx`, `AuthCard.test.tsx`, `Stepper.test.tsx` (49 tests) | une régression de comportement sur un axe que §5.1 attribue à un appelant mesuré | **corriger le code.** Si c'est l'appelant qui a changé, re-lire le callsite dans `mika-cloud` et mettre §5.1 à jour **avant** de toucher l'assertion — c'est la mesure qui gouverne, pas le test |
| les 8 assertions `axe` des trois fichiers | une violation d'accessibilité sur un état rendu | **corriger le rendu.** `packages/ui/CLAUDE.md` § Accessibility Standards fait de l'assertion manquante un review-fail : la retirer n'est pas une option disponible |
| `theme.test.ts` § LC.2b (22 assertions) | une règle `.mika-*` qui cesse de composer les tokens §2, perd le rayon §6, supprime l'anneau de focus, ou cache la classe `sr-only` d'une manière qui la retire aussi de l'arbre d'accessibilité | **corriger la CSS.** Ces assertions disent ce que les classes *signifient* ; le test de composant ne dit que quelle classe est émise |
| l'anti-vacuité globale de `theme.test.ts` (plancher `>= 32`) | un scan devenu aveugle — préfixe renommé, règles disparues — qui se lirait comme un fichier propre | **établir quelles règles ont quitté le fichier.** Ne pas baisser le plancher pour faire passer le build : un plancher qui cesse de suivre le fichier est un plancher qui cesse de garder |

**Contrôle négatif exécuté, et non supposé.** Les deux assertions les plus
porteuses ont été vérifiées par mutation avant d'être retenues : passer
`type="submit"` au contrôle reveal, et remplacer le repli `current` hors liste par
l'index brut. Exactement trois tests rougissent, et ce sont les trois attendus.
Sans cette vérification, le test de soumission de formulaire aurait pu être vide
de sens — il dépend de ce que jsdom émet réellement au clic.

**Disposition du plan lui-même :** la halte-et-remontée de mika#1574 option (c)
a été **réalisée** — l'implémentation s'est arrêtée deux fois et est remontée à
l'opérateur, qui a levé la précondition au troisième tour. Elle est éteinte.

## 9. Vérification

Deux moitiés : les mesures de la halte, et la vérification du code qui l'a suivie.

### 9.1 Vérification du code livré

```bash
npm test      --prefix packages/ui   # 350 tests, 21 fichiers
npm run typecheck --prefix packages/ui
npm run build --prefix packages/ui
npm run build --prefix dashboard     # le consommateur bâtit toujours
```

**Contrôle positif et négatif de la décision de présentation**, qui est la seule
partie non évidente : les règles `theme.css` doivent atteindre le CSS bâti d'un
consommateur, là où une utilitaire Tailwind écrite dans `packages/ui` ne
l'atteint pas.

```bash
grep -o -E 'mika-sr-only|mika-field--secret|mika-auth-card|mika-stepper__connector' \
  dashboard/dist/assets/index-*.css | sort | uniq -c   # attendu : non vide
grep -c -E '\.w-9[,{ ]' dashboard/dist/assets/index-*.css   # attendu : 0
```

Mesuré le 2026-09-28 : les quatre classes sont présentes, `.w-9` est absente —
donc la mesure fondatrice de LC.2 tient toujours sur cet arbre, et c'est elle qui
justifie que la présentation ne soit pas écrite en utilitaires.

### 9.2 Les mesures de la halte, conservées

Chacune est une commande, et chacune rendait le résultat annoté **tant que le
blocage correspondant tenait**. Elles sont conservées pour que la levée reste
vérifiable dans les deux sens.

```bash
# B1 — aucune des trois sections de rulebook           [attendu : 0 ligne]
grep -niE 'password|secret field|auth card|centered card|stepper' \
  docs/design/luminescent-core.md

# B1′ — aucun callsite, et mika-cloud hors d'atteinte  [attendu : 0 ligne, puis absent]
grep -rniE 'type="password"|PasswordInput|SecretField|AuthCard|CenteredCard|<Stepper' \
  --include='*.tsx' --include='*.ts' . | grep -v node_modules
ls -d /data/workspace/mika-platform/mika-cloud   # attendu : No such file or directory

# B2 — LEVÉ : la mesure a changé de sens, et c'est le contrôle de la levée
git log --oneline -1 origin/main                 # attendu : fab69e13 … (mika#1801) (#2556)
ls packages/ui/src/components/Button.tsx         # attendu : le fichier existe
grep '"version"' packages/ui/package.json        # attendu : 0.4.0, jamais 0.3.1
```

**Contrôle négatif de la halte elle-même.** Les deux premières commandes rendant
`0 ligne`, leur silence ne prouve rien s'il vient d'une faute de frappe plutôt que
d'une absence. Avant de conclure, vérifier que le `grep` regarde bien quelque
chose : `grep -c '^#' docs/design/luminescent-core.md` doit rendre un compte non
nul (27 au dispatch d'exécution), et `ls packages/ui/src/components/ | wc -l` un
inventaire non vide (35 fichiers, 17 primitives et leurs tests). *Un scan
silencieusement inerte se lit exactement comme un arbre propre.*

**Cinq haltes à la relecture de ce plan :**

- **Halte 1 — les commentaires du ticket ne m'ont pas été livrés. RÉALISÉE, et
  c'est elle qui a débloqué le ticket.** `gh` n'était authentifié dans aucun des
  trois bacs à sable successifs (`gh auth status` → *not logged into any GitHub
  hosts*), et le contexte injecté ne portait que le corps. Cette halte prescrivait
  ceci, mot pour mot : *« si un commentaire opérateur tient g1 ou g2 … alors §2.2
  est périmé et ce plan doit être révisé plutôt que suivi »*. **C'est exactement ce
  qui s'est produit** — un commentaire opérateur du 2026-09-28T16:04:52Z a déposé
  trois appelants réels, et ce plan a été révisé (§0, §5.1) au lieu d'être suivi.
  Elle avait aussi tiré la bonne conséquence de conduite : aucun re-dispatch ne
  l'aurait levée, et aucun ne l'a levée — c'est un geste extérieur qui l'a fait.
  Conservée parce qu'une halte qu'on supprime cesse d'expliquer pourquoi le
  déblocage devait venir du dehors.
- **Halte 2 — si une mesure de §9 cesse de rendre le résultat attendu**, le blocage
  correspondant est levé : ne pas re-dispatcher à l'aveugle, relire §6 pour savoir
  s'il restait cumulatif avec un autre. **Cette halte s'est réalisée** entre le
  grooming et l'exécution, sur B2 ; §0 est le produit de sa relecture, et §6 dit
  ce que la levée ne dispense pas de faire.
- **Halte 3 — ne pas lever B2 en rebasant ce ticket sur `feat/1801`. Sans objet
  désormais.** Elle interdisait d'empiler ce travail sur une branche non revue de
  13 commits, où un `hold[review]` sur LC.2 aurait emporté les deux. LC.2 est
  mergé : il n'y a plus de branche sur quoi empiler, et l'ordre prescrit — `main`
  ← LC.2, puis LC.2b — est tenu. Conservée parce qu'une halte qu'on supprime cesse
  d'expliquer pourquoi l'ordre était le bon.
- **Halte 4 — ne pas lire « B2 est levé » comme « le ticket est débloqué ».
  Toujours vraie, et toujours utile, mais ce n'est plus B2 qui décide.** Elle
  gardait contre un contresens précis : B2 était cumulatif, jamais alternatif, et
  le lever ne fournissait ni section de rulebook ni callsite. C'est **B1′** qui a
  débloqué, et seulement lui. Un lecteur qui ouvrirait §0 aujourd'hui, verrait
  trois lignes vertes et en conclurait que tout a été satisfait se tromperait
  encore : **B1 n'est pas tenu**, il est devenu *sans objet* parce que la
  précondition est une disjonction. La différence compte pour le rulebook — §6
  dit pourquoi le manque reste ouvert et remonté plutôt que comblé ici.
- **Halte 5 — ne pas lire l'absence de règle d'enforcement comme un oubli.**
  Le point 4 du périmètre dit « **seulement si** la primitive a une population à
  garder », et les trois n'en ont aucune dans ce dépôt : leur consommateur est
  `mika-cloud`. Une huitième règle dans `packages/ui/CLAUDE.md` passerait au vert
  tous les jours sans jamais rien regarder, et son silence se lirait comme une
  conformité — classe mika#2205. Le motif est écrit dans `packages/ui/CLAUDE.md`
  lui-même, pour que le prochain relecteur trouve la décision et non le vide.

## 10. Definition of Done

- [x] La précondition du ticket est mesurée voie par voie, avec les commandes qui
      la reproduisent (§2).
- [x] L'inexécutabilité structurelle de g2 depuis un dispatch `mika` est établie
      et écrite, avec sa conséquence de conduite (§2.2).
- [x] Le second blocage — LC.2 non mergé, recouvrement de trois fichiers — est
      mesuré et nommé (§3).
- [x] L'affirmation périmée du corps (« LC.2 a livré `<Button>` et `<Spinner>` »)
      est corrigée par la mesure (§3), **puis rendue exacte par le merge de LC.2**
      (§3.3) — le corps n'appelle plus de correction sur ce point.
- [x] Les trois blocages sont **re-mesurés au dispatch d'exécution**, chacun avec
      sa commande et son verdict daté ; B2 est constaté levé, B1 et B1′ tenant
      (§0).
- [x] Les sections que la levée de B2 périme (§3, §6, §7, §9) sont redatées plutôt
      qu'effacées, la mesure du grooming étant conservée avec la sienne (§3).
- [x] Aucune API n'est proposée pour les trois primitives, et le refus est motivé
      primitive par primitive (§5).
- [x] Le geste qui lève chaque blocage est nommé avec son propriétaire, et
      l'ordre cumulatif est dit (§6).
- [x] Le périmètre d'exécution futur est transcrit, chaque point annoté de sa
      dépendance (§7).
- [x] La non-applicabilité du gate mika#2306 est déclarée avec sa raison (§8).
- [x] Les **cinq** haltes de relecture sont écrites, dont la limite de ma propre
      mesure (Halte 1, **réalisée** — c'est elle qui a débloqué le ticket) et le
      contresens que la levée de B2 rend possible (Halte 4) — §9.
- [x] **La halte a tenu jusqu'à ce que la précondition soit tenue**, et pas un
      tour de plus : deux dispatches ont rendu le même refus, le troisième a
      trouvé g2 satisfaite et a exécuté.
- [x] Les axes d'API sont **dérivés des appelants**, un par un, chacun citant le
      fichier qui le fixe (§5.1) — y compris les deux que §5 n'avait pas anticipés
      (l'erreur rendue par l'appelant, l'étape courante hors de la liste).
- [x] Les cinq points du périmètre sont livrés (§7), avec **zéro règle
      d'enforcement** et le motif écrit.
- [x] Le manque de rulebook (B1) est **remonté** à Vincent dans
      `packages/ui/CLAUDE.md`, pas comblé ici — §8 du rulebook le lui réserve.

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria`. Les critères AC1 à AC9
ci-dessous ont été dérivés de sa précondition et portaient **sur la remontée**.
Ils sont conservés parce qu'ils ont gouverné deux dispatches et que leur
satisfaction est ce qui a rendu la halte défendable ; **trois d'entre eux
(AC4, AC5, AC7) sont explicitement périmés par la levée** et le disent en place.
Les critères de l'extraction elle-même sont ceux du §7, et AC10 à AC12 ci-dessous
les complètent.

1. **AC1 — la précondition est tranchée par mesure, pas par citation.** Les deux
   voies g1 et g2 sont chacune évaluées par une commande reproductible dont le
   résultat est donné, et la mesure est datée de l'instant du dispatch
   (`HEAD == origin/main`, aucun retard).
2. **AC2 — l'inexécutabilité de g2 depuis `mika` est établie.** Le plan démontre
   que `mika-cloud` n'est pas atteignable depuis le bac à sable de dispatch, et en
   tire la conséquence : aucun re-drive sur `mika` ne peut tenir g2, seul un dépôt
   du callsite dans ce dépôt ou dans le ticket peut le faire.
3. **AC3 — le second blocage est nommé.** Le plan établit que mika#1801 n'était
   pas mergé au grooming, que son recouvrement avec le périmètre de ce ticket
   portait sur les trois fichiers `packages/ui/src/index.ts`,
   `packages/ui/CLAUDE.md` et `packages/ui/package.json`, et que ce blocage était
   cumulatif avec la précondition plutôt qu'alternatif. **Sa levée au dispatch
   d'exécution est mesurée et datée** (§0, §3.3), et la nature cumulative est
   réaffirmée là où elle décide : le lever ne satisfait ni g1 ni g2.
4. **AC4 — aucune API n'est inventée. ~~Tenu~~ → PÉRIMÉ par la levée, et
   remplacé par AC10.** Il était tenu tant que la précondition ne l'était pas :
   le refus était motivé pour chacune des trois en citant l'axe qu'un callsite
   aurait révélé. g2 tenue, ces axes **sont** révélés, et ne pas les écrire serait
   devenu le défaut inverse. §5.1 les écrit, chacun avec le fichier qui le fixe.
5. **AC5 — aucune règle d'enforcement sans population. Tenu, et il survit à la
   levée** — c'est le seul des trois. Il portait sur l'absence de ligne dans
   `packages/ui/CLAUDE.md` tant qu'aucune primitive n'existait ; il vaut encore
   après extraction, pour la raison que le plan donnait déjà : les consommateurs
   vivent dans `mika-cloud`, donc la population reste nulle **dans ce dépôt**. Le
   tableau des primitives gagne trois lignes — ce n'est pas une règle
   d'enforcement — et la section « Enforcement Rules » n'en gagne aucune, avec le
   motif écrit à l'endroit où un relecteur cherchera l'oubli.
6. **AC6 — la remontée est actionnable.** Chaque blocage porte le geste qui le
   lève, son propriétaire, et la mention explicite qu'aucun des trois n'est
   exécutable par un pilote dispatché sur `mika`.
7. **AC7 — l'arbre est inchangé hors de ce fichier. ~~Tenu~~ → PÉRIMÉ par la
   levée, et remplacé par AC11.** Il était le contrôle mécanique de la halte :
   tant que la précondition n'était pas tenue, le diff ne devait contenir que ce
   plan. Il l'a été sur deux dispatches. Le périmètre s'étant ouvert, le diff
   touche désormais `packages/ui/` — et `dashboard/`, `site/`, `crates/` restent
   intacts, ce qui est la part de AC7 qui garde un sens et que AC11 reprend.
8. **AC8 — la limite de la mesure est écrite.** Le plan déclare que les
   commentaires du ticket ne lui ont pas été livrés (`gh` non authentifié) et que
   la lecture d'un commentaire tenant g1 ou g2 périme §2.2. **La tentative a été
   refaite au dispatch d'exécution et a échoué à l'identique** ; la limite est
   donc celle des deux instants, pas d'un accident du grooming.
9. **AC9 — la halte est re-mesurée avant d'être maintenue, et non reconduite par
   citation.** Les trois blocages sont rejoués à l'instant du dispatch
   d'exécution ; le verdict de chacun est donné avec sa commande ; les sections
   qu'une levée périme sont redatées plutôt qu'effacées ; et le contresens que
   cette levée rend possible — lire un blocage tombé comme un ticket débloqué —
   est nommé en Halte 4. Le verdict d'ensemble était inchangé à ces deux
   instants ; il a changé au troisième, et §0 le mesure de la même façon.
10. **AC10 — chaque axe d'API cite l'appelant qui le fixe.** §5.1 reprend une par
    une les questions que §5 posait comme non répondables, et donne pour chacune
    le fichier `mika-cloud` qui la tranche. Deux axes que §5 n'avait pas anticipés
    y figurent aussi : l'erreur de champ rendue par l'appelant, et l'étape
    courante qui peut légitimement ne pas être dans la liste affichée. **Aucune
    union discriminée n'est livrée**, et c'est le résultat de la mesure, pas une
    omission : chaque axe candidat est tranché identiquement par l'appelant
    unique, donc une union y serait une interface sous plusieurs noms — la
    décoration que D1 de LC.2 refuse.
11. **AC11 — le diff est borné à `packages/ui/` et à ce plan.** Aucun fichier sous
    `dashboard/`, `site/` ou `crates/` n'est touché. L'adoption des trois
    primitives par une surface est LC.3, comme le corps du ticket le dit.
12. **AC12 — la présentation atteint réellement un consommateur.** Les règles
    `.mika-*` ajoutées à `theme.css` sont vérifiées présentes dans le CSS bâti du
    dashboard, et le contrôle négatif (`.w-9`, écrite uniquement dans
    `packages/ui`, absente) confirme que la mesure fondatrice de LC.2 tient
    toujours. Sans ce couple, « la primitive est stylée » et « la primitive est
    stylée dans un fichier que personne ne compile » se lisent identiquement.

## 11. Hors périmètre, délibérément

- ~~**L'extraction des trois primitives.**~~ **Livrée** (§7) — la précondition
  est tenue depuis la levée de B1′.
- **L'adoption customer-facing** — c'est LC.3, comme le corps le dit déjà.
- **Une quatrième variante de `<Button>`** (`confirm` / `destructive`) — décision
  produit §8, ticket séparé si Vincent la veut (LC.2 D4).
- **Merger mika#1801, ou le rebaser sur ce ticket.** Le merge appartenait à la
  boucle QA et à l'opérateur ; l'empilement était explicitement refusé (§9,
  halte 3). **Fait depuis, par la boucle** (`fab69e13`, PR #2556) — et dans le bon
  ordre, ce qui rend la halte 3 sans objet plutôt que contournée.
- **Ajouter au rulebook les sections manquantes.** §8 du rulebook les réserve à
  Vincent. Les écrire ici serait le contournement de procédure que §2.1 nomme, et
  la voie g1 cesserait d'être une levée de précondition pour devenir une invention
  de plus, signée d'une autre main.
- **Migrer les appelants de `mika-cloud` vers les trois primitives.** C'est LC.3,
  et c'est un autre dépôt. Ce qui est livré ici est ce que LC.3 consommera ; rien
  dans `mika-cloud` n'est touché, et l'API est de ce fait **non vérifiée en
  service** tant que cette migration n'a pas eu lieu. C'est la limite honnête de
  ce ticket : les axes sont dérivés d'appelants lus, pas d'appelants compilés.
- **Toute règle d'enforcement pour les trois primitives.** Point 4 du périmètre,
  clause « seulement si » — voir Halte 5 et la note de `packages/ui/CLAUDE.md`.
  Elle appartient au dépôt qui porte la surface.
