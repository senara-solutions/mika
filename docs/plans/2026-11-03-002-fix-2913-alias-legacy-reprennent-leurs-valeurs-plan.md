# mika#2913 — les alias « backward-compat » reprennent leurs valeurs 0.3.1, et les deux surfaces in-repo re-aliasent vers le canon

**Ticket :** senara-solutions/mika#2913 (`bug`, `p2-normal`, `ready`, `dispatch:loop`)
**Corps de PR :** `Closes #2913`, `Refs #2912`
**Chemin retenu :** celui que Vincent a nommé le 2026-10-26 (« une regression. je n'ai rien changé »), avec son défaut écrit — les cinq alias reprennent leurs valeurs 0.3.1, `--color-error` et `--color-surface-container-high` gardent leur valeur canonique, et `bannedLegacyHexValues` est réécrite dans l'autre sens, jamais supprimée sans remplaçant.

---

## 1. Le défaut, re-mesuré sur `5623bec53`

Le bloc en cause, `packages/ui/src/theme.css` l. 55-65 :

```css
  /* Backward-compat aliases (legacy names → canonical values).
     Ensures any consumer missed by the packages/ui + dashboard grep sweep
     continues to render (with the new canonical color — the intended fix). */
  --color-bg: var(--color-background);        /* #0c0e11, était #0d0f12 */
  --color-bg-card: var(--color-surface-container);  /* #171a1d, était #151820 */
  --color-accent: var(--color-primary);       /* #ada3ff, était #7c6af7 */
  --color-accent-light: var(--color-secondary);     /* #9d8fff, inchangé */
  --color-heading: var(--color-on-surface);   /* #e8e8ec, était #e8ecf2 */
  --color-muted: var(--color-on-surface-variant);   /* #aaabaf, était #a0a8b8 */
```

La table du ticket est exacte. Le commentaire du bloc dit lui-même ce que Vincent
désavoue : « with the new canonical color — **the intended fix** ». Et le plan de
mika#1800 l'écrit encore plus nettement (l. 211) : « it now renders with the *new*
canonical color instead of the legacy shade. Mitigation: **this IS the intended
fix** ». L'aliasing n'était pas un oubli : c'était le vecteur délibéré par lequel
le canon §2 devait atteindre les consommateurs sans migrer leurs callsites.

### 1.1 Deux faits mesurés que le corps du ticket n'a pas

Le corps dit : « Le rulebook ne les nomme pas, et **la lib ne les lit pas pour ses
propres composants** ». Les deux moitiés de la phrase ne tiennent pas de la même
façon, et la seconde est fausse — ce qui déplace le périmètre.

**Fait A — la lib lit ses propres alias, 39 fois hors tests, dans 13 primitives.**
Mesuré sur `packages/ui/src` (hors `*.test.tsx`) :

| primitive | ce qu'elle lit |
|---|---|
| `StatusBadge` | `bg-accent/10`, `text-accent`, `bg-accent` (variante `info`) ; `text-muted`, `bg-muted` (`neutral`) |
| `MarkdownContent` | `text-muted`, `text-heading` ×5, `text-accent`, `hover:text-accent-light`, `border-accent/30` |
| `LoadingState` | `bg-bg-card` ×3 |
| `EmptyState` | `text-heading`, `text-accent`, `hover:text-accent-light`, `bg-bg-card`, `text-muted/60` ×2 |
| `ErrorState` | `text-muted/70`, `text-heading`, `text-muted/60` |
| `ListRow` | `outline-accent/40` ×2, `text-accent/40`, `text-muted/30` |
| `SelectFilter` | `bg-bg`, `text-muted`, `focus:border-accent/40` |
| `LiveRefreshToggle` | `text-muted`, `bg-accent` (track) |
| `CopyButton` | `ring-accent`, `ring-offset-bg` |
| `Pagination`, `CostMeter`, `TokenBudgetBar`, `utils/badges.ts` | `text-muted` (×6), `bg-bg-card` |

**Fait B — les deux surfaces in-repo lisent ces alias ~975 fois, sur 791 lignes.**

| alias | `dashboard/src` | `site/src` |
|---|---|---|
| `muted` | 409 | 45 (+3 `bg-muted`) |
| `heading` | 114 | 12 |
| `accent` | 143 (98 `text`, 31 `bg`, 13 `border`, 1 `ring`) | 57 (24 `bg`, 21 `text`, 6 `border`, 3 `shadow`, 2 `outline`, 1 `ring`) |
| `accent-light` | 83 | 4 |
| `bg-card` | 74 | 10 |
| `bg` | 17 | 5 |

Et `site/src/index.css` l. 8-11 porte la promesse écrite que ce vol invalide :
« `theme.css` still exports … as backward-compat aliases onto the canonical
values, so every `bg-accent` / `text-muted` utility in this app keeps working and
**now resolves to canon** ».

### 1.2 Ce que ces deux faits imposent au geste

Basculer les alias en valeurs legacy **et ne rien faire d'autre** ne répare pas
une régression : il en déplace une. Trois conséquences, chacune mesurée :

1. **Le dashboard et le landing de ce dépôt se dé-canonisent**, sur 791 lignes
   qu'aucun vol ne relit. C'est le défaut du ticket, à l'envers, sur deux surfaces
   dont le rulebook est la loi.
2. **Le « bug des 2 primaires » ressuscite.** `<Button variant="primary">` peint
   `.mika-cta-gradient`, qui compose `var(--color-primary)` / `var(--color-primary-dim)`
   — des noms **canoniques**, que ce vol ne touche pas. Un dashboard dont les
   `text-accent` passent à #7c6af7 pendant que son CTA reste #ada3ff affiche deux
   violets côte à côte : exactement ce que mika#1800 avait pour mission de faire
   disparaître (son plan l. 152 : « le ticket's spirit ("2-primaires bug
   disparaît") »).
3. Migrer les 791 lignes vers les noms canoniques coûterait ~700 lignes — **2,5×
   la borne par vol**, pour un résultat purement mécanique.

Le geste doit donc séparer ce que l'aliasing confondait : **la valeur qu'une lib
publiée sert à un consommateur non-migré**, et **la valeur qu'une surface
canonique de ce dépôt lit**. C'est ce que fait le § 2.

---

## 2. La décision : la lib rend les alias au legacy, les deux surfaces in-repo re-aliasent vers le canon

Les six lignes `--color-accent: var(--color-primary)` ne **disparaissent** pas :
elles **se déplacent**, de la lib vers les deux consommateurs in-repo qui les
voulaient.

```
packages/ui/src/theme.css      : --color-accent: #7c6af7;              ← valeur 0.3.1, littérale
dashboard/src/index.css        : --color-accent: var(--color-primary);  ← le canon, localement
site/src/index.css             : --color-accent: var(--color-primary);  ← le canon, localement
```

Ce que cela produit, surface par surface :

| surface | ce qu'elle reçoit sur un nom legacy | pourquoi c'est juste |
|---|---|---|
| `mika-cloud/web/src` (862 usages, non migré) | **legacy** | Le nom promettait la compatibilité ; il la tient. C'est la demande de Vincent. |
| `dashboard/src` (840 usages) | **canon** | Surface sous rulebook. Zéro dé-canonisation, zéro double-primaire, zéro ligne migrée. |
| `site/src` (136 usages) | **canon** | Idem. Et le commentaire l. 4-15 redevient vrai, par un mécanisme local. |
| un consommateur neuf n'écrivant que du canon | canon | Les noms canoniques ne bougent pas. |

Trois propriétés qui font retenir cette forme plutôt qu'une migration :

- **Elle est un no-op visuel sur ce dépôt.** Le dashboard et le landing rendent
  après exactement ce qu'ils rendaient avant. Un vol qui répare un consommateur
  sans toucher au pixel de deux autres est un vol dont la revue tient en une table.
- **Elle rend la migration des 791 lignes non urgente.** Sans elle, le suivi porte
  une fenêtre de double-primaire ouverte sur deux surfaces ; avec elle, le suivi
  n'est plus que du nettoyage de nommage, sans échéance visuelle. (§ 6, `Tracked in:`)
- **Elle tient sous la borne** : 182 lignes estimées contre 279 (§ 9).

### 2.1 Pourquoi la lib garde ses primitives sur les noms legacy (et ne migre pas)

Le fait A pourrait suggérer de migrer les 13 primitives vers les noms canoniques.
**Ce serait le mauvais geste, et il casserait précisément ce que Vincent demande.**

Tailwind ne scanne pas `packages/ui` — `theme.css` le documente, et les tests le
citent (`theme.test.ts` l. 233-237). Les utilitaires que les primitives écrivent
sont donc générés **par le consommateur**, avec la valeur que *ce* consommateur
donne au nom. `<StatusBadge variant="info">` rend donc #ada3ff dans le dashboard
(qui re-aliase) et #7c6af7 dans mika-cloud (qui ne re-aliase pas) — un seul code,
deux valeurs, chacune celle que sa surface a choisie. Migrer les primitives vers
`text-primary` servirait du canon à mika-cloud **dans les primitives**, alors
qu'il réclame le legacy : la migration annulerait la moitié du correctif.

Que les primitives d'une lib soient écrites sur des noms déclarés « legacy » reste
une dette de structure — préexistante à ce vol, qui n'en change que la valeur.
Nommée et tracée au § 6.

### 2.2 Les deux tokens canoniques ne bougent pas

`--color-error` reste #ff6e84 et `--color-surface-container-high` reste #1d2024 :
ce sont des tokens **canoniques**, nommés par le rulebook §2, pas des alias.
C'est le défaut écrit du ticket (« sans mot contraire »), et pour `error` le
corps dit pourquoi : restaurer #ef4444 reviendrait sur `76a8b0de` (#1976) **dans
le rulebook**, qui est hors de portée d'un vol de code (§8 réserve le rulebook à
Vincent). Les deux hex legacy correspondants restent donc interdits — ils sont le
contenu résiduel du détecteur D2 (§ 3.2).

### 2.3 Six alias, pas cinq — et le sixième est un no-op

Le ticket dit « les cinq alias », et note justement que `--color-accent-light`
n'a pas bougé (`var(--color-secondary)` = #9d8fff = sa valeur 0.3.1). Le geste le
traite quand même, en écrivant `#9d8fff` littéral côté lib et
`var(--color-secondary)` côté consommateurs. **Aucun pixel ne change.** La raison
est au § 3.2 : les détecteurs D1 et D3 asservissent le bloc *entier* à une seule
forme, et un sixième membre laissé dans l'autre forme serait le cas spécial par
lequel la garde cesse d'être uniforme. Un bloc homogène se vérifie ; un bloc à
exception se discute.

---

## 3. Le geste

### 3.1 `packages/ui/src/theme.css` — le bloc alias (l. 55-65)

Les six déclarations passent en hex littéral 0.3.1, et le commentaire est réécrit
pour porter le contrat **inversé** : ce bloc est l'énoncé de ce qu'un nom legacy
vaut, pour toujours, et non une étape vers le canon.

```css
  /* Backward-compat aliases — legacy names, LEGACY values (mika#2913).
     La compatibilité porte sur la VALEUR, pas seulement sur le nom. Entre 0.3.1
     et 0.5.0 ces six noms ont pris les valeurs canoniques du rulebook §2
     (LC.1, mika#1800), et un consommateur qui ne faisait que monter de version a
     vu toutes ses pages changer de couleur — mesuré sur mika-cloud#287, 862
     usages. Vincent l'a qualifié de régression le 2026-10-26.

     Ces noms sont donc gelés sur leurs valeurs 0.3.1, en LITTÉRAL et jamais en
     `var(--color-*)` : un `var()` vers un token canonique est exactement le
     mécanisme par lequel la régression est arrivée, et il la rejouerait au
     prochain mouvement du §2.

     Les surfaces de CE dépôt sont sous rulebook et veulent le canon : elles
     re-aliasent ces six noms vers les tokens §2 chez elles
     (`dashboard/src/index.css`, `site/src/index.css`), ce qui est asservi par
     `theme.test.ts § les consommateurs in-repo re-aliasent`. Migrer leurs 791
     lignes vers les noms canoniques est tracé à part — le re-alias rend ce
     nettoyage sans échéance visuelle, pas superflu.

     `--color-error` et `--color-surface-container-high` ne sont PAS ici : ce sont
     des tokens canoniques §2, lus par la lib pour ses propres composants, et ils
     gardent leur valeur canonique (ticket § « sans mot contraire »). */
  --color-bg: #0d0f12;
  --color-bg-card: #151820;
  --color-accent: #7c6af7;
  --color-accent-light: #9d8fff;  /* no-op : identique à var(--color-secondary) */
  --color-heading: #e8ecf2;
  --color-muted: #a0a8b8;
```

Le commentaire du bloc §2 « canonical error triad » (l. 39-45) nomme
`bannedLegacyHexValues` comme ce qui bannit #ef4444 ; cette phrase reste vraie
après § 3.2 et n'est pas touchée.

### 3.2 `packages/ui/src/theme.test.ts` — la garde, réécrite dans l'autre sens

La garde actuelle a trois tables. `canonicalTokens` (l. 52-88) ne bouge pas : le
§ 2.2 est précisément la décision de ne pas y toucher. Les deux autres sont
réécrites, et deux volets nouveaux les empêchent de stagner.

**D1 — `backwardCompatAliases` → `legacyAliasValues` (remplace l. 90-97 et le
`it.each` l. 154-161).** La table passe de `alias → var(--color-*)` à
`alias → hex 0.3.1`, et l'assertion reste un `.toBe()` sur `declarationMap`,
donc inchangée dans sa forme : `last-wins` continue de couvrir le cas de la
déclaration dupliquée. C'est le remplaçant direct, dans l'autre sens — un
consommateur qui écrit un nom legacy reçoit la valeur legacy, et un `var()`
réintroduit par mégarde échoue en nommant la dérive (`expected "#7c6af7",
received "var(--color-primary)"`).

**D2 — `bannedLegacyHexValues` réduit à deux entrées (l. 103-114).** Les cinq
valeurs que D1 exige désormais en sortent : les y laisser ferait rougir le vol
contre lui-même. Restent exactement les hex dont le nom est **canonique** et dont
la valeur canonique n'est pas restaurée :

```ts
const bannedLegacyHexValues: Array<[hex: string, wasFor: string]> = [
  ['#1e2130', 'legacy surface-container-high — le nom est canonique §2, valeur #1d2024 (mika#2913 § 2.2)'],
  ['#ef4444', 'former §5.5 error hex — retiré du rulebook par #1976 ; le restaurer reviendrait sur le rulebook, pas sur la lib'],
]
```

Le scan reste `expect(THEME_CSS).not.toContain(hex)` sur le fichier entier
(commentaires strippés) : les deux hex y sont bien absents après § 3.1.

**D3 — l'assertion auto-nettoyante de D2.** Une entrée de D2 devient *stale* le
jour où son hex redevient canonique — un rulebook qui restaurerait #ef4444
rendrait l'interdiction fausse, et la garde continuerait de passer en interdisant
la bonne valeur. Le volet croise donc les deux tables :

```ts
it.each(bannedLegacyHexValues)(
  'banned hex %s is not the value of a canonical token (anti-stale)',
  (hex) => {
    // Une entrée dont le hex est redevenu canonique n'est plus une
    // interdiction : c'est une garde qui rougirait contre le rulebook. Ce
    // croisement la fait rougir ELLE, et force la suppression de l'entrée.
    expect(canonicalTokens.map(([, h]) => h.toLowerCase())).not.toContain(hex.toLowerCase())
  },
)
```

Le symétrique couvre D1 : une valeur legacy qui coïnciderait avec la valeur
canonique de son homologue rendrait l'entrée tautologique (l'alias ne prouverait
plus rien). Même forme, sur `legacyAliasValues`.

**D4 — l'exhaustivité du bloc (volet nouveau).** D1 est une liste ; une **septième**
déclaration ajoutée au bloc sans entrée dans la liste passerait en silence — la
classe de défaut que `check-landing-tokens.sh` a dû quitter une liste pour
attraper, et que `theme.test.ts` cite déjà à son propre sujet (l. 486-491 : « a
denylist … is green on a *new* purple written in good faith »). Le volet est
donc une règle de **forme sur le bloc**, pas sur les valeurs :

- localiser la région du bloc dans `THEME_CSS_RAW` (les commentaires y sont
  intacts) : du commentaire `Backward-compat aliases` au commentaire suivant ;
- toute déclaration `--color-*` de cette région doit (i) avoir une entrée dans
  `legacyAliasValues`, et (ii) porter un hex littéral, jamais un `var(`.

Les deux sens sont asservis — une entrée de `legacyAliasValues` qui ne
correspondrait plus à aucune déclaration rougit aussi, ce qui fait expirer la
table avec le bloc qu'elle décrit.

**D5 — les consommateurs in-repo re-aliasent (volet nouveau).** C'est la
contrepartie du § 2 : si `dashboard/src/index.css` ou `site/src/index.css` perdait
son bloc de re-alias, la surface se dé-canoniserait en silence, sur 791 lignes que
personne ne relit. Le volet lit les deux fichiers et asserte que chacun des six
noms y est redéclaré vers le token §2 attendu.

Il vit dans `theme.test.ts`, et non dans un script CI ni dans un test du
dashboard, pour trois raisons : (i) ce fichier est le site où le contrat des alias
est écrit, donc le site où sa contrepartie s'asserte ; (ii) `site/` n'a **aucun
runner de test** (`site/package.json` ne déclare pas `test`), donc une garde
posée là-bas ne tournerait nulle part — la classe mika#2578/#2752 exactement ;
(iii) `npm test -w packages/ui` est déjà requis en CI (`ci.yml` l. 302), donc le
volet est câblé par construction et ne touche pas `scripts/suite-wiring.tsv`.
La lecture hors-package se fait par `readFileSync` en relatif, comme l. 22 le
fait déjà pour `theme.css`.

### 3.3 `dashboard/src/index.css` et `site/src/index.css` — le re-alias

Après les deux `@import` existants, un bloc `@theme` qui redéclare les six noms
vers leurs tokens §2 :

```css
@theme {
  /* Les six noms legacy, re-aliasés vers le canon §2 POUR CETTE SURFACE
     (mika#2913). `@samidarko/ui/theme.css` les gèle désormais sur leurs valeurs
     0.3.1, parce qu'un consommateur non migré qui monte de version ne doit pas
     voir ses pages changer de couleur (mika-cloud#287). Cette surface, elle, est
     sous rulebook et veut le canon : 840 utilitaires `text-muted` / `bg-accent` /
     … sont écrits sur ces noms (136 côté `site/`) et résolvent ici vers les
     tokens §2.
     Asservi par `packages/ui/src/theme.test.ts`.
     Migrer ces utilitaires vers les noms canoniques est tracé à part ; d'ici là
     ce bloc est ce qui garde la surface canonique, et il est un no-op visuel. */
  --color-bg: var(--color-background);
  --color-bg-card: var(--color-surface-container);
  --color-accent: var(--color-primary);
  --color-accent-light: var(--color-secondary);
  --color-heading: var(--color-on-surface);
  --color-muted: var(--color-on-surface-variant);
}
```

Côté `site/`, le commentaire l. 4-15 est réécrit : sa phrase « now resolves to
canon » reste vraie, mais le mécanisme a changé de fichier, et c'est exactement ce
qu'un lecteur doit apprendre là.

Deux contraintes à respecter à l'implémentation :

- **Aucun hex littéral sous `site/src/`.** `scripts/check-landing-tokens.sh` (job
  `landing-tokens-lint`) refuse toute couleur littérale sous `site/src/` et
  `site/index.html`. Le re-alias s'écrit en `var(--color-*)`, ce qui satisfait le
  lint par construction — et c'est aussi la bonne forme : un hex recopié ici
  cesserait de suivre le §2, la régression de mika#1804 un fichier plus loin.
- **L'override cross-`@import` se vérifie, il ne se suppose pas.** Tailwind v4
  fusionne les blocs `@theme`, dernier gagnant, et la forme
  `--color-accent: var(--color-primary)` dans `@theme` est déjà prouvée (c'est
  l'état actuel de `theme.css`). Le seul inconnu est que la redéclaration arrive
  après un `@import`. Vérification : `npm run build -w dashboard`, puis lire
  `dashboard/dist/assets/*.css` et constater que `--color-accent` y résout vers
  #ada3ff et non #7c6af7. **Repli nommé** si l'override ne prend pas : déclarer
  les six redéfinitions dans `:root` au lieu de `@theme`. Les utilitaires
  `text-accent` sont déjà générés par le `@theme` de la lib ; seule la valeur de
  la variable change, ce qui suffit — et D5 s'écrit alors sur `:root`.

### 3.4 Ce qui ne bouge pas

- `canonicalTokens` (29 entrées) et les valeurs §2 — § 2.2.
- `.mika-*` et leurs assertions (LC.2, LC.2b) : elles composent des tokens
  canoniques, jamais un alias. Le volet « carries no colour literal » (l. 504-511)
  est scopé sur `mikaRules`, donc les hex du bloc alias ne l'atteignent pas.
- Les 13 primitives de `packages/ui/src` — § 2.1.
- Les 791 lignes de `dashboard/src` + `site/src` — § 6.
- `packages/ui/package.json` (0.5.0) : le bump et la publication npm sont hors
  vol, gestes de Vincent, suivis dans mika#2912 AC3. À nommer dans le corps de
  PR : un changement de valeur de token sur une lib 0.x appelle 0.6.0.
- `docs/design/luminescent-core.md` : §8 réserve le rulebook à Vincent, et ce vol
  ne le contredit pas — il ne touche aucun token qu'il nomme.
- `scripts/check-cta-primitives.sh` : son prédicat est une forme (« solid fill »),
  pas une liste de classes, et ses exclusions portent sur le rôle, la
  translucidité et l'élément — toutes invariantes par ce vol, qui ne renomme
  aucune classe. Allowlist inchangée, donc toujours vide et pinnée vide.

---

## 4. Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés du périmètre que Vincent a nommé le 2026-11-03 et des § 2-3.

- **AC1** — `packages/ui/src/theme.css` déclare les six alias du bloc
  « Backward-compat » en hex littéral 0.3.1 : `--color-bg` #0d0f12,
  `--color-bg-card` #151820, `--color-accent` #7c6af7, `--color-accent-light`
  #9d8fff, `--color-heading` #e8ecf2, `--color-muted` #a0a8b8. Aucun `var(` dans
  le bloc.
- **AC2** — `--color-error` vaut #ff6e84 et `--color-surface-container-high` vaut
  #1d2024 : inchangés. Les 29 entrées de `canonicalTokens` passent sans
  modification de la table.
- **AC3** — `bannedLegacyHexValues` existe toujours, réduite à `#1e2130` et
  `#ef4444`, chaque entrée nommant sa raison et son ticket. La garde n'a pas été
  supprimée, et aucun de ses deux hex n'apparaît dans `theme.css`.
- **AC4** — `theme.test.ts` asserte positivement les six valeurs legacy (D1),
  et une déclaration du bloc sans entrée dans la table — ou portant un `var(` —
  fait rougir la suite (D4), dans les deux sens.
- **AC5** — une entrée de `bannedLegacyHexValues` dont le hex est devenu la valeur
  d'un token de `canonicalTokens` fait rougir la suite (D3), de même qu'une valeur
  de `legacyAliasValues` coïncidant avec la valeur canonique de son homologue.
- **AC6** — `dashboard/src/index.css` et `site/src/index.css` redéclarent les six
  noms vers leurs tokens §2, et le retrait de l'un d'eux fait rougir
  `npm test -w packages/ui` (D5).
- **AC7** — no-op visuel sur ce dépôt, vérifié sur le CSS buildé : après
  `npm run build -w dashboard`, `--color-accent` résout vers #ada3ff dans
  `dashboard/dist`. Idem pour `site`.
- **AC8** — suites vertes : `npm test -w packages/ui`, `npm test -w dashboard`,
  `npm run typecheck -w packages/ui`, `npm run lint -w dashboard`,
  `npm run build -w packages/ui`, `npm run build -w dashboard`,
  `npm run build -w site`, `scripts/check-landing-tokens.sh`,
  `scripts/check-cta-primitives.sh`, `scripts/verify-pipeline.sh origin/main`.
- **AC9** — hors vol, et nommé comme tel dans le corps de PR : publication npm,
  bump de version, rebase de mika-cloud#287 (mika#2912 AC3).

---

## 5. Fire-Disposition

Le plan livre des détecteurs — D1 à D5 du § 3.2 sont tous du code dont le chemin
de succès est « aucune violation trouvée ». Option retenue : **(a) exception
nommée en allowlist, dégénérée à zéro entrée.**

**Pourquoi zéro.** Les cinq détecteurs atterrissent **armés et verts**, sans
exception, parce que le même commit écrit la conformité qu'ils exigent : D1/D4 sur
le bloc que § 3.1 réécrit, D5 sur les deux `index.css` que § 3.3 écrit, D2/D3 sur
deux hex vérifiés absents du fichier. Il n'existe aucune violation préexistante à
exempter, donc aucune liste d'exceptions à ouvrir. Inventer une allowlist vide
ici serait du cérémonial : `scripts/cta-primitives-allowlist.txt` ship vide parce
que sa population est **réelle** et pourrait croître ; la population de D1-D5 est
un bloc de six lignes dans un fichier, close par D4.

**Où la substance de (a) s'applique quand même.** Les deux entrées *de contenu* de
D2 (`#1e2130`, `#ef4444`) sont, elles, des interdictions nommées qui survivent à
leur cause : chacune nomme la donnée précise, référence son ticket (mika#2913 § 2.2,
#1976), et **porte son assertion auto-nettoyante** — D3 les fait rougir le jour où
leur hex redevient canonique, c'est-à-dire le jour où l'interdiction cesse d'être
vraie. D4 joue le même rôle pour D1 : une table qui cesserait de décrire le bloc
rougit dans les deux sens. Aucune de ces entrées ne peut devenir stale en silence.

**Ce qui n'est pas désarmé, et pourquoi.** Aucun détecteur n'atterrit avec
`.skip` ni `#[ignore]` : l'option (b) existe pour un détecteur qui rougirait sur
une population qu'on ne peut pas assainir dans le vol, ce qui n'est pas le cas
ici. Et aucun ne justifie (c) : le périmètre est écrit par Vincent et le § 2 ne
demande aucun cadrage supplémentaire.

---

## 6. Hors vol, et tracé

- **Migrer les 791 lignes de `dashboard/src` + `site/src`** des six noms legacy
  vers les noms canoniques (`text-muted` → `text-on-surface-variant`,
  `bg-accent` → `bg-primary`, …), puis retirer les deux blocs de re-alias du
  § 3.3 et le volet D5 avec eux. ~700 lignes, soit 2,5× la borne par vol : à
  découper par surface (dashboard ~600, site ~100), voire par alias. Le re-alias
  du § 3.3 est ce qui rend ce nettoyage **sans échéance visuelle** : d'ici là les
  deux surfaces restent canoniques.
- **Les 13 primitives de `packages/ui/src` écrites sur des noms legacy** (§ 2.1,
  fait A). Dette de structure préexistante : une lib dont les primitives lisent
  des noms qu'elle déclare legacy fait dépendre leur couleur du consommateur.
  C'est aujourd'hui la propriété qui fait marcher le § 2 — donc le geste n'est
  *pas* une migration vers le canon, mais une décision à prendre sur ce qu'une
  primitive doit servir à un consommateur neuf.
- **Publication npm, bump 0.6.0, rebase de mika-cloud#287** : gestes de Vincent,
  mika#2912 AC3.
- **La correction du CTA primaire** : mika#2912, et ce vol ne l'inclut pas —
  Vincent n'a pas nommé son choix dans mika#2912 avant ce groom.

Tracked in: senara-solutions/mika#2912

---

## 7. Risques

| risque | ampleur | mitigation |
|---|---|---|
| L'override `@theme` cross-`@import` ne prend pas | bloquant pour AC6/AC7 | Repli nommé au § 3.3 : les six redéfinitions dans `:root`. Vérifié sur le CSS buildé, pas supposé. |
| Un hex littéral glissé dans `site/src/index.css` | `landing-tokens-lint` rouge | Le re-alias s'écrit en `var()` par construction (§ 3.3). |
| D5 lit deux fichiers hors de son package | chemins relatifs fragiles au déplacement | Même mécanisme que l. 22 (`readFileSync` relatif à `import.meta.url`) ; un déplacement casse le test au lieu de le rendre vacant. |
| Un test du dashboard assertant une classe legacy | `npm test -w dashboard` rouge | Aucune classe n'est renommée par ce vol ; les seules assertions de classe legacy mesurées sont `Button.test.tsx` l. 86 (`not.toContain('bg-accent')`) et `Spinner.test.tsx` l. 41-42 (`text-muted` passé en prop), toutes deux insensibles à un changement de **valeur**. |
| mika-cloud reçoit le legacy dans les primitives de la lib | voulu (§ 2.1), mais plus large que ce que le corps du ticket décrit | Nommé dans le corps de PR comme conséquence mesurée du fait A, avec la table du § 1.1. |

---

## 8. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `packages/ui/src/theme.css` (bloc alias + commentaire du contrat) | 18 |
| `packages/ui/src/theme.test.ts` (D1-D4 : table, assertions, anti-stale, exhaustivité) | 86 |
| `packages/ui/src/theme.test.ts` (D5 : les deux consommateurs in-repo re-aliasent) | 32 |
| `dashboard/src/index.css` (bloc `@theme` de re-alias + commentaire) | 22 |
| `site/src/index.css` (bloc `@theme` de re-alias + réécriture du commentaire l. 4-15) | 24 |

Total estimé : 182 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 182 contre 279 : 97 lignes de marge, et la nature
du travail est favorable — quatre fichiers, aucun nouveau, aucun module Rust,
aucune migration de callsite. Le poste le plus lourd (118 lignes de
`theme.test.ts`) est de la table et de l'assertion sur un fichier déjà lu par le
même test, pas de la logique neuve. Les 700 lignes de migration qui auraient fait
exploser la borne sont explicitement renvoyées au § 6, avec `Tracked in:`, et le
re-alias du § 3.3 est ce qui autorise ce renvoi sans laisser de fenêtre visuelle
ouverte.

---

## 9. Definition of Done

- [ ] Les six alias de `theme.css` portent leur hex 0.3.1 littéral ; aucun `var(`
      dans le bloc (AC1).
- [ ] `--color-error` #ff6e84 et `--color-surface-container-high` #1d2024
      inchangés ; `canonicalTokens` non modifiée (AC2).
- [ ] `bannedLegacyHexValues` conservée et réduite à `#1e2130` + `#ef4444`, chaque
      entrée nommant sa raison et son ticket (AC3).
- [ ] D1 et D4 livrés armés et verts ; une septième déclaration du bloc, ou un
      `var(`, fait rougir la suite (AC4).
- [ ] D3 livré : une entrée bannie devenue canonique fait rougir la suite, et
      symétriquement sur `legacyAliasValues` (AC5).
- [ ] D5 livré : les deux `index.css` re-aliasent les six noms, et le retrait de
      l'un fait rougir `npm test -w packages/ui` (AC6).
- [ ] No-op visuel vérifié sur le CSS buildé du dashboard et du site : les six
      noms résolvent vers les valeurs §2 (AC7).
- [ ] Suites vertes, lints inclus (AC8), et `scripts/verify-pipeline.sh origin/main`.
- [ ] `## Fire-Disposition` du § 5 reflétée dans le corps de PR.
- [ ] Corps de PR : `Closes #2913`, `Refs #2912` ; nomme le hors-vol du § 6 (npm,
      bump 0.6.0, rebase mika-cloud#287), le fait A du § 1.1 et sa conséquence sur
      les primitives, et `Code review: delegated to mika-qa (mika#2694)`.
- [ ] `Tracked in: senara-solutions/mika#2912`.
