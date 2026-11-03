# mika#2913 — les alias « backward-compat » reprennent leurs valeurs 0.3.1, et le CTA primaire est aplati sous la règle §5 amendée

**Ticket :** senara-solutions/mika#2913 (`bug`, `p2-normal`, `ready`, `dispatch:loop`)
**Corps de PR :** `Closes #2913`, `Refs #2912`
**Périmètre : deux volets, un seul vol, une seule PR.** C'est le périmètre écrit en tête du corps le 2026-11-03 à 07:40Z (dormance levée) et en tête de mika#2912 à 07:53Z :

- **Volet A — la régression des alias.** Les cinq alias reprennent leurs valeurs 0.3.1, `--color-error` et `--color-surface-container-high` gardent leur valeur canonique, et `bannedLegacyHexValues` est réécrite dans l'autre sens, jamais supprimée sans remplaçant.
- **Volet B — AC1 et AC2 de mika#2912**, suivant la **voie (c)** que Prime a rendue sur délégation de Vincent : amender le §5 de `docs/design/luminescent-core.md` dans la même PR, avec la règle citée là-bas, et sans toucher au §7.

**Hors vol :** la publication npm, le bump de version et le rebase de mika-cloud#287 — gestes de Vincent, suivis dans mika#2912 AC3. **Le merge de l'amendement du rulebook passe par le mot de Vincent.**

> Une note de lecture, parce qu'elle décide de la recevabilité du volet B. `CLAUDE.md` écrit que le rulebook « is owned by Vincent and updated via direct commits, not PRs; implementation PRs apply it but do not relitigate it ». Le volet B **amende le rulebook dans une PR d'implémentation**, ce qui est l'exception et non la règle : elle est écrite, datée et signée (choix de voie (c) délégué par Vincent à Prime, mika#2912 en tête, 2026-11-03 07:53Z), et le §8 est satisfait dans l'ordre qu'il prescrit — le motif est remonté à Vincent (étape 2), accepté (étape 3), et le merge reste suspendu à son mot. Ce plan n'invente aucune section et ne relitige rien : il transcrit une règle déjà rendue.

---

## 1. Volet A — le défaut des alias, re-mesuré sur `5623bec53`

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
2. **Deux violets cohabiteraient.** Un dashboard dont les `text-accent` passent à
   #7c6af7 pendant que tout ce qui compose `var(--color-primary)` reste à #ada3ff
   — `.mika-btn--tertiary` (l. 261), le focus ring (l. 196), `--color-surface-tint`
   — affiche deux primaires côte à côte : exactement ce que mika#1800 avait pour
   mission de faire disparaître (son plan l. 152 : « le ticket's spirit
   ("2-primaires bug disparaît") »). Le volet B déplace le CTA hors de `primary`,
   mais ne ferme pas cette liste : les autres sites restent.
3. Migrer les 791 lignes vers les noms canoniques coûterait ~700 lignes — **2,5×
   la borne par vol**, pour un résultat purement mécanique.

Le geste doit donc séparer ce que l'aliasing confondait : **la valeur qu'une lib
publiée sert à un consommateur non-migré**, et **la valeur qu'une surface
canonique de ce dépôt lit**. C'est ce que fait le § 2.

---

## 2. Volet A — la décision : la lib rend les alias au legacy, les deux surfaces in-repo re-aliasent vers le canon

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

- **Elle est un no-op visuel sur ce dépôt** pour le volet A. Le dashboard et le
  landing rendent après exactement ce qu'ils rendaient avant sur les six noms.
  (Le volet B, lui, change délibérément un pixel : le CTA. § 4.)
- **Elle rend la migration des 791 lignes non urgente.** Sans elle, le suivi porte
  une fenêtre de double-primaire ouverte sur deux surfaces ; avec elle, le suivi
  n'est plus que du nettoyage de nommage, sans échéance visuelle. (§ 7, `Tracked in:`)
- **Elle tient sous la borne**, volet B compris : 255 lignes estimées contre 279 (§ 9).

### 2.1 Pourquoi la lib garde ses primitives sur les noms legacy (et ne migre pas)

Le fait A pourrait suggérer de migrer les 13 primitives vers les noms canoniques.
**Ce serait le mauvais geste, et il casserait précisément ce que Vincent demande.**

Tailwind ne scanne pas `packages/ui` — `theme.css` le documente (l. 136-151), et
les tests le citent. Les utilitaires que les primitives écrivent sont donc générés
**par le consommateur**, avec la valeur que *ce* consommateur donne au nom.
`<StatusBadge variant="info">` rend donc #ada3ff dans le dashboard (qui re-aliase)
et #7c6af7 dans mika-cloud (qui ne re-aliase pas) — un seul code, deux valeurs,
chacune celle que sa surface a choisie. Migrer les primitives vers `text-primary`
servirait du canon à mika-cloud **dans les primitives**, alors qu'il réclame le
legacy : la migration annulerait la moitié du correctif.

Que les primitives d'une lib soient écrites sur des noms déclarés « legacy » reste
une dette de structure — préexistante à ce vol, qui n'en change que la valeur.
Nommée et tracée au § 7.

### 2.2 Les deux tokens canoniques ne bougent pas

`--color-error` reste #ff6e84 et `--color-surface-container-high` reste #1d2024 :
ce sont des tokens **canoniques**, nommés par le rulebook §2, pas des alias.
C'est le défaut écrit du ticket (« sans mot contraire »), et pour `error` le
corps dit pourquoi : restaurer #ef4444 reviendrait sur `76a8b0de` (#1976) **dans
le rulebook**, ce que la voie (c) n'autorise pas — elle ouvre le §5 sur la règle
du CTA, pas le §2 sur la triade `error`. Les deux hex legacy correspondants
restent donc interdits : ils sont le contenu résiduel du détecteur D2 (§ 5.2).

### 2.3 Six alias, pas cinq — et le sixième est un no-op

Le ticket dit « les cinq alias », et note justement que `--color-accent-light`
n'a pas bougé (`var(--color-secondary)` = #9d8fff = sa valeur 0.3.1). Le geste le
traite quand même, en écrivant `#9d8fff` littéral côté lib et
`var(--color-secondary)` côté consommateurs. **Aucun pixel ne change.** La raison
est au § 5.2 : les détecteurs D1 et D4 asservissent le bloc *entier* à une seule
forme, et un sixième membre laissé dans l'autre forme serait le cas spécial par
lequel la garde cesse d'être uniforme. Un bloc homogène se vérifie ; un bloc à
exception se discute.

---

## 3. Volet B — le défaut du CTA, et ce que la règle amendée impose

### 3.1 Le fait, et pourquoi il n'a pas de solution locale

`.mika-cta-gradient` (l. 112-128) peint `--color-on-surface` (#e8e8ec) sur un
dégradé 135° de `--color-primary` (#ada3ff) à `--color-primary-dim` (#715eeb).
Contrastes WCAG 2.x (mika#2912, recalculés ici) :

| texte | fond | contraste | seuil AA |
|---|---|---|---|
| #e8e8ec | #ada3ff (`primary`) | **1,82:1** | ✗ |
| #e8e8ec | #715eeb (`primary_dim`) | **3,82:1** | ✗ |
| #0c0e11 (texte foncé, piste écartée) | #715eeb | 4,14:1 | ✗ |

Aucun arrêt n'atteint 4,5:1, et le texte foncé ne sauve pas l'arrêt sombre — le
corps de mika#2912 le dit, et le §7 interdit de toute façon de quitter
`on_surface` pour du blanc. Le défaut est **dans la combinaison des deux règles**
du rulebook, §5 (dégradé imposé) et §7 (texte clair imposé), pas dans un
consommateur. C'est pourquoi la voie retenue corrige la règle.

### 3.2 La règle amendée, et l'unique teinte que le §2 autorise

La règle que Prime a rendue, et que le §5 doit porter :

> un dégradé n'est admis sur un CTA que si le texte `on_surface` atteint 4,5:1 sur
> chaque arrêt, hover compris ; sinon le CTA est aplati sur une teinte unie qui
> satisfait le seuil.

Appliquée à l'état du §2 : le dégradé tombe, et il faut une teinte unie. Le
critère est arithmétique — `on_surface` a une luminance relative de 0,8093, donc
**un fond satisfait 4,5:1 si et seulement si sa luminance relative est ≤ 0,1409**.
Balayage des tokens §2 de la famille primaire/secondaire :

| token | hex | L | contraste vs `on_surface` |
|---|---|---|---|
| `--color-primary` | #ada3ff | 0,432 | 1,82:1 ✗ |
| `--color-primary-container` / `-fixed` | #9f93ff | — | ✗ (plus clair que `primary_dim`) |
| `--color-primary-dim` | #715eeb | 0,175 | 3,82:1 ✗ |
| `--color-secondary` / `-dim` | #9d8fff | — | ✗ |
| **`--color-secondary-container`** | **#4434a0** | **0,062** | **7,66:1 ✓** |

**`--color-secondary-container` est le seul token §2 de la famille violette qui
passe le seuil**, et il le passe largement (AAA, qui est à 7:1). Il n'y en a donc
pas à choisir : il n'y a pas d'arbitrage, il y a un unique candidat. Inventer un
hex neuf serait ajouter un token au §2, ce que le §8 réserve à Vincent et que la
voie (c) n'a pas ouvert.

**Le hover.** §5 ne prescrit aucun hover pour le primary ; l'actuel est une
décision d'implémentation, et son intention écrite (l. 119) est « a shift toward
`--color-secondary`, i.e. **darker** ». On la conserve littéralement en mélangeant
vers le fond, ce qui assombrit et donc **monte** le contraste du texte :

```css
background-color: color-mix(in srgb, var(--color-secondary-container) 88%, var(--color-background));
```

Soit #3d2f8f, L = 0,050, **8,59:1** — satisfait par construction, et aucun hex
littéral n'entre dans une règle `.mika-` (ce que `theme.test.ts` l. 504-511
interdit absolument).

### 3.3 Le prix, mesuré et nommé

La décision de Prime dit « le prix est assumé ». Il n'est pas seulement la perte
du dégradé — il y a un second terme, qu'aucun des deux tickets ne chiffre et que
ce plan mesure :

| | avant (0.5.0) | après |
|---|---|---|
| texte sur le CTA | **1,82:1** – 3,82:1 | **7,66:1** (hover 8,59:1) |
| CTA contre le fond de page #0c0e11 | 8,71:1 | **2,07:1** |

Le vol **répare le contraste du texte et dégrade celui de la silhouette**. WCAG
1.4.11 demande 3:1 pour les parties visuelles qui identifient un composant ; un
bouton rempli dont le libellé est à 7,66:1, de 1,5 rem de rayon, reste identifié
par son libellé, mais la silhouette passe sous ce seuil et le fait est réel.

Il n'a **pas** de remède dans ce vol : la fenêtre qui satisferait les deux
(0,113 ≤ L ≤ 0,141) ne contient **aucun** token §2 — `secondary_container` est à
0,062, `primary_dim` à 0,175, rien entre les deux. La satisfaire demanderait un
token neuf au §2, c'est-à-dire une décision §8 de Vincent. Ce plan ne la prend
pas : il la nomme, la chiffre, et la trace (§ 7). Le corps de PR la porte, parce
que c'est l'information dont Vincent a besoin pour dire son mot sur l'amendement.

### 3.4 Le nom de la classe ne change pas

`.mika-cta-gradient` ne peindra plus de dégradé, et le nom mentira. Il est
pourtant conservé : c'est une **classe CSS publique** du paquet (les
consommateurs l'écrivent via `<Button variant="primary">`, et rien n'empêche de
l'écrire à la main), et la renommer serait un *breaking change* d'API non demandé,
au milieu d'un vol dont le sujet est précisément qu'un changement non annoncé a
cassé un consommateur. Le renommage touche de surcroît `Button.tsx`,
`ErrorState.tsx` et 4 tests. Le commentaire du bloc nomme le mensonge, et le
renommage est tracé au § 7.

---

## 4. Le geste

### 4.1 `packages/ui/src/theme.css` — le bloc alias (l. 55-65)

Les six déclarations passent en hex littéral 0.3.1, et le commentaire est réécrit
pour porter le contrat **inversé** : ce bloc est l'énoncé de ce qu'un nom legacy
vaut, pour toujours, et non une étape vers le canon. Il doit dire, en ~10 lignes
(le budget commentaire est resserré par rapport aux 22 lignes d'un plan à un seul
volet — § 9) :

- que la compatibilité porte sur la **valeur** et pas seulement sur le nom, et que
  la bascule de 0.3.1 à 0.5.0 a recoloré un consommateur entier (mika-cloud#287,
  862 usages), ce que Vincent a qualifié de régression le 2026-10-26 ;
- que les valeurs sont écrites en **littéral et jamais en `var(--color-*)`** : un
  `var()` vers un token canonique est exactement le mécanisme par lequel la
  régression est arrivée, et il la rejouerait au prochain mouvement du §2 ;
- que les surfaces de ce dépôt veulent le canon et le **re-aliasent chez elles**
  (`dashboard/src/index.css`, `site/src/index.css`), asservi par D5 ;
- que `--color-error` et `--color-surface-container-high` ne sont **pas** ici
  (§ 2.2).

```css
  --color-bg: #0d0f12;
  --color-bg-card: #151820;
  --color-accent: #7c6af7;
  --color-accent-light: #9d8fff;  /* no-op : identique à var(--color-secondary) */
  --color-heading: #e8ecf2;
  --color-muted: #a0a8b8;
```

Le commentaire du bloc §2 « canonical error triad » (l. 39-45) nomme
`bannedLegacyHexValues` comme ce qui bannit #ef4444 ; cette phrase reste vraie
après § 5.2 et n'est pas touchée.

### 4.2 `packages/ui/src/theme.css` — le CTA (l. 96-128)

`.mika-cta-gradient` passe de `background-image: linear-gradient(...)` à
`background-color: var(--color-secondary-container)`, et son hover à la forme
`color-mix` du § 3.2. Le commentaire de tête (l. 96-111) est réécrit : sa phrase
porteuse — « §2 Signature Textures + §5 Buttons : main CTAs are a `primary` ->
`primary_dim` linear gradient at 135deg » — **cesse d'être la règle** et doit
céder la place à la règle §5 amendée, citée avec son ticket, ses deux contrastes
avant (1,82 / 3,82) et son contraste après (7,66 / 8,59). La justification de la
*localisation* (pourquoi cette texture vit dans `theme.css` et non dans le TSX)
reste vraie mot pour mot et se conserve.

`transition` (l. 185-190) liste déjà `background-image` **et**
`background-color` : rien à y changer. `--radius-mika-xl`, `border: 0` et
`color: var(--color-on-surface)` de `.mika-btn--primary` sont inchangés — §5 et §7
les prescrivent toujours, et l'amendement ne touche pas le §7.

### 4.3 `docs/design/luminescent-core.md` — l'amendement du §5 (hors compte de lignes)

Sous la ligne 105 (« **Primary:** Gradient fill (`primary` to `primary_dim`) … »),
la règle du § 3.2 est écrite telle que Prime l'a rendue, avec :

- le seuil (4,5:1, texte `on_surface`, **chaque** arrêt, hover compris) ;
- la conséquence (sinon, aplatissement sur une teinte unie satisfaisant le seuil) ;
- l'état d'application à la date : `primary`→`primary_dim` ne satisfait pas, donc
  le CTA est aplati sur `secondary_container` ;
- la provenance (mika#2912, voie (c), délégation de Vincent à Prime le 2026-11-03)
  et le gardien permanent (`theme.test.ts`, D6).

**Le §7 n'est pas touché** — c'est une contrainte du choix, pas une préférence.
Le §2 « Signature Textures » (l. 42-45) n'est pas touché non plus : il décrit le
dégradé comme texture de la marque, ce qui reste vrai des surfaces sans texte ;
seule son application aux CTA est désormais conditionnée par le §5.

### 4.4 `dashboard/src/index.css` et `site/src/index.css` — le re-alias

Après les deux `@import` existants, un bloc `@theme` qui redéclare les six noms
vers leurs tokens §2, avec un commentaire court disant pourquoi la lib ne le fait
plus, ce que la surface y gagne (840 et 136 utilitaires qui restent canoniques),
et qu'il est asservi par `packages/ui/src/theme.test.ts` :

```css
@theme {
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

### 4.5 Ce qui ne bouge pas

- `canonicalTokens` (29 entrées) et les valeurs §2 — § 2.2. En particulier
  `--color-secondary-container` garde #4434a0 : le volet B le **consomme**, il ne
  le redéfinit pas.
- Le §7 du rulebook, et `.mika-btn--primary` (`border: 0`, texte `on_surface`).
- Les 13 primitives de `packages/ui/src` — § 2.1.
- Les 791 lignes de `dashboard/src` + `site/src` — § 7.
- `Button.tsx`, `ErrorState.tsx` et leurs tests : la classe garde son nom (§ 3.4),
  et les quatre assertions mesurées portent sur la **présence** de la classe
  (`Button.test.tsx` l. 84/102/115, `ErrorState.test.tsx` l. 49/58), jamais sur sa
  valeur de fond.
- `packages/ui/package.json` (0.5.0) : le bump et la publication npm sont hors
  vol, gestes de Vincent, suivis dans mika#2912 AC3. À nommer dans le corps de
  PR : deux changements de valeur de token sur une lib 0.x appellent 0.6.0.
- `scripts/check-cta-primitives.sh` : son prédicat est une forme (« solid fill »),
  pas une liste de classes, et ses exclusions portent sur le rôle, la
  translucidité et l'élément — toutes invariantes par ce vol, qui ne renomme
  aucune classe et n'ajoute aucun fond hors `theme.css`. Allowlist inchangée,
  donc toujours vide et pinnée vide.

---

## 5. Les détecteurs

### 5.1 Volet A — D1, D4, D5 : la garde des alias, réécrite dans l'autre sens

La garde actuelle a trois tables. `canonicalTokens` (l. 52-88) ne bouge pas : le
§ 2.2 est précisément la décision de ne pas y toucher.

**D1 — `backwardCompatAliases` → `legacyAliasValues` (remplace l. 90-97 et le
`it.each` l. 154-161).** La table passe de `alias → var(--color-*)` à
`alias → hex 0.3.1`, et l'assertion reste un `.toBe()` sur `declarationMap`, donc
inchangée dans sa forme : `last-wins` continue de couvrir le cas de la déclaration
dupliquée. C'est le remplaçant direct, dans l'autre sens — un consommateur qui
écrit un nom legacy reçoit la valeur legacy, et un `var()` réintroduit par mégarde
échoue en nommant la dérive (`expected "#7c6af7", received "var(--color-primary)"`).

**D4 — l'exhaustivité du bloc.** D1 est une liste ; une **septième** déclaration
ajoutée au bloc sans entrée dans la liste passerait en silence — la classe de
défaut que `check-landing-tokens.sh` a dû quitter une liste pour attraper, et que
`theme.test.ts` cite déjà à son propre sujet (l. 486-491 : « a denylist … is green
on a *new* purple written in good faith »). Le volet est donc une règle de **forme
sur le bloc**, pas sur les valeurs : localiser la région dans `THEME_CSS_RAW` (les
commentaires y sont intacts) du commentaire `Backward-compat aliases` au
commentaire suivant, puis asserter que toute déclaration `--color-*` de cette
région a une entrée dans `legacyAliasValues` **et** porte un hex littéral, jamais
un `var(`. Les deux sens sont asservis — une entrée de la table qui ne
correspondrait plus à aucune déclaration rougit aussi, ce qui fait expirer la
table avec le bloc qu'elle décrit.

**D5 — les consommateurs in-repo re-aliasent.** C'est la contrepartie du § 2 : si
`dashboard/src/index.css` ou `site/src/index.css` perdait son bloc, la surface se
dé-canoniserait en silence sur 791 lignes que personne ne relit. Le volet lit les
deux fichiers et asserte que chacun des six noms y est redéclaré vers le token §2
attendu.

Il vit dans `theme.test.ts`, et non dans un script CI ni dans un test du
dashboard, pour trois raisons : (i) ce fichier est le site où le contrat des alias
est écrit, donc le site où sa contrepartie s'asserte ; (ii) `site/` n'a **aucun
runner de test** (`site/package.json` ne déclare pas `test`), donc une garde posée
là-bas ne tournerait nulle part — la classe mika#2578/#2752 exactement ;
(iii) `npm test -w packages/ui` est déjà requis en CI, donc le volet est câblé par
construction et ne touche pas `scripts/suite-wiring.tsv`. La lecture hors-package
se fait par `readFileSync` en relatif, comme l. 22 le fait déjà pour `theme.css`.

### 5.2 Volet A — D2 et D3 : la denylist réduite, et son auto-nettoyage

**D2 — `bannedLegacyHexValues` réduit de 7 à 2 entrées (l. 103-114).** Les cinq
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
(commentaires strippés) : les deux hex y sont bien absents après § 4.1.

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

### 5.3 Volet B — D6 : le gardien permanent de la règle §5 amendée (AC2)

C'est le livrable que mika#2912 AC2 demande nommément, et le texte de la décision
dit sa fonction : « le test d'AC2 devient le gardien permanent de cette règle ».
Il ne vérifie donc pas « le CTA est `secondary_container` » — ce serait épingler
une valeur, et la valeur n'est pas la règle. Il vérifie **la règle** :

1. extraire le fond du CTA depuis `ruleFor('.mika-cta-gradient')` et de son hover,
   sous les deux formes possibles — une valeur unique (`background-color`) ou la
   liste des arrêts d'un `linear-gradient` (`background-image`) ;
2. résoudre chaque arrêt en hex via `declarationMap` ;
3. lire la couleur du texte depuis `ruleFor('.mika-btn--primary')` (`on_surface`),
   plutôt que de la recopier : le jour où le §7 bougerait, le test suivrait ;
4. asserter contraste ≥ 4,5 sur **chaque** arrêt, rest et hover.

Écrit ainsi, le détecteur réadmet un dégradé le jour où les teintes de marque le
permettent, exactement comme la règle le prévoit — et il rougit si l'une d'elles
redescend sous le seuil, ce qu'AC2 demande mot pour mot.

Le résolveur de couleur accepte **trois formes et trois seulement** : hex
littéral, `var(--token)` (résolu récursivement par `declarationMap`), et
`color-mix(in srgb, <A> P%, <B>)` (interpolation sRGB composante par composante,
ce que la spec prescrit pour cet espace). **Toute autre forme le fait jeter**, et
le test rougit. Ce point est le cœur du détecteur, pas un détail
d'implémentation : un résolveur qui rendrait « indéterminé → vert » déclarerait
conforme le premier `oklch()` ou `light-dark()` écrit de bonne foi, et c'est
précisément la silencieuse-conformité que ce dépôt refuse. Un cas de test
l'asserte directement, sur une expression inventée dans le test.

**Le rouge-avant est obligatoire et il est daté.** AC2 écrit « Le rouge-avant se
fait sur 0.5.0 ». Procédure, dans cet ordre : écrire D6 **avant** le § 4.2, le
faire tourner sur l'arbre inchangé, et constater qu'il rougit en imprimant 1,82 et
3,82 ; puis appliquer le § 4.2 et constater le vert à 7,66 et 8,59. Les deux
sorties vont dans le corps de PR. Un détecteur dont on n'a pas vu le rouge n'a
pas été vu détecter.

**Le périmètre de D6 est le CTA primaire, et c'est une borne volontaire.** Il
serait tentant de l'élargir à toute règle `.mika-*` qui déclare un fond **et** une
couleur de texte. Ce serait ouvrir une population non mesurée (input fields, auth
card, stepper, les hovers) au milieu d'un vol déjà à deux volets, avec un risque
de rouge sur du préexistant hors périmètre — et la Fire-Disposition serait alors
(a) avec de vraies entrées, ou (c). La paire mesurée au passage :
`.mika-btn--secondary:hover` peint `secondary_container` sous `on_surface`, soit
les 7,66:1 du § 3.2, donc conforme. L'élargissement est tracé au § 7.

**D7 — le describe « CTA texture » (l. 199-227) est réécrit, pas supprimé.** Ses
trois `it` épinglent aujourd'hui `linear-gradient`, `135deg` et l'ordre
`primary` avant `primary_dim` : ils énoncent la règle d'avant l'amendement et
rougiraient sur le § 4.2. Ils sont réécrits pour épingler ce que le §5 amendé
prescrit — présence de la règle de repos, hover toujours scopé hors `:disabled`
(la raison technique de `:not(:disabled)` est inchangée et reste assertée), et
fond composé de tokens. Comme D2, **la garde n'est pas retirée sans remplaçant** :
c'est le même geste, un volet plus loin.

---

## 6. Acceptance criteria

Le corps de mika#2913 ne porte pas de section `## Acceptance criteria` : AC1-AC7
ci-dessous sont dérivés du périmètre que Vincent a nommé le 2026-11-03 et des
§ 2-5. **AC8 et AC9 sont la transcription verbatim d'AC1 et AC2 de mika#2912**,
qui entrent dans ce vol ; AC10 transcrit son AC3, qui en est exclu.

- **AC1** — `packages/ui/src/theme.css` déclare les six alias du bloc
  « Backward-compat » en hex littéral 0.3.1 : `--color-bg` #0d0f12,
  `--color-bg-card` #151820, `--color-accent` #7c6af7, `--color-accent-light`
  #9d8fff, `--color-heading` #e8ecf2, `--color-muted` #a0a8b8. Aucun `var(` dans
  le bloc.
- **AC2** — `--color-error` vaut #ff6e84 et `--color-surface-container-high` vaut
  #1d2024 : inchangés. Les 29 entrées de `canonicalTokens` passent sans
  modification de la table, `--color-secondary-container` #4434a0 incluse.
- **AC3** — `bannedLegacyHexValues` existe toujours, réduite à `#1e2130` et
  `#ef4444`, chaque entrée nommant sa raison et son ticket. La garde n'a pas été
  supprimée, et aucun de ses deux hex n'apparaît dans `theme.css`.
- **AC4** — `theme.test.ts` asserte positivement les six valeurs legacy (D1), et
  une déclaration du bloc sans entrée dans la table — ou portant un `var(` — fait
  rougir la suite (D4), dans les deux sens.
- **AC5** — une entrée de `bannedLegacyHexValues` dont le hex est devenu la valeur
  d'un token de `canonicalTokens` fait rougir la suite (D3), de même qu'une valeur
  de `legacyAliasValues` coïncidant avec la valeur canonique de son homologue.
- **AC6** — `dashboard/src/index.css` et `site/src/index.css` redéclarent les six
  noms vers leurs tokens §2, et le retrait de l'un d'eux fait rougir
  `npm test -w packages/ui` (D5).
- **AC7** — no-op visuel du volet A sur ce dépôt, vérifié sur le CSS buildé :
  après `npm run build -w dashboard`, `--color-accent` résout vers #ada3ff dans
  `dashboard/dist`. Idem pour `site`.
- **AC8** *(= mika#2912 AC1, verbatim)* — « le texte du CTA primaire atteint 4,5:1
  sur **chaque** arrêt du dégradé, hover compris (`.mika-cta-gradient:hover`,
  `-25%` / `85%`) ». Satisfait par aplatissement (§ 3.2) : la règle §5 amendée
  n'admet le dégradé que si le seuil est tenu, et il ne l'est pas — il n'y a donc
  plus d'arrêt de dégradé, et le fond unique du repos (7,66:1) comme celui du
  hover (8,59:1) passent le seuil.
- **AC9** *(= mika#2912 AC2, verbatim)* — « un test de la lib calcule ce contraste
  à partir des variables de `theme.css`, et échoue si l'une d'elles redescend sous
  le seuil. Le rouge-avant se fait sur 0.5.0. » Livré par D6, rouge-avant inclus
  dans le corps de PR, et une forme de couleur non résoluble fait rougir plutôt
  que passer.
- **AC10** *(= mika#2912 AC3)* — le §5 de `docs/design/luminescent-core.md` porte
  la règle amendée avec sa provenance ; le §7 et le §2 sont inchangés. **Le merge
  de la PR attend le mot de Vincent sur cet amendement**, et le corps de PR le dit.
- **AC11** — suites vertes : `npm test -w packages/ui`, `npm test -w dashboard`,
  `npm run typecheck -w packages/ui`, `npm run lint -w dashboard`,
  `npm run build -w packages/ui`, `npm run build -w dashboard`,
  `npm run build -w site`, `scripts/check-landing-tokens.sh`,
  `scripts/check-cta-primitives.sh`, `scripts/verify-pipeline.sh origin/main`.
- **AC12** — hors vol, et nommé comme tel dans le corps de PR : publication npm,
  bump 0.6.0, rebase de mika-cloud#287 (mika#2912 AC3, gestes de Vincent).

---

## 7. Hors vol, et tracé

- **Migrer les 791 lignes de `dashboard/src` + `site/src`** des six noms legacy
  vers les noms canoniques (`text-muted` → `text-on-surface-variant`,
  `bg-accent` → `bg-primary`, …), puis retirer les deux blocs de re-alias du
  § 4.4 et le volet D5 avec eux. ~700 lignes, soit 2,5× la borne par vol : à
  découper par surface (dashboard ~600, site ~100), voire par alias. Le re-alias
  du § 4.4 est ce qui rend ce nettoyage **sans échéance visuelle**.
- **Le contraste de la silhouette du CTA (2,07:1 contre le fond, § 3.3).** Sans
  remède dans le §2 actuel : la fenêtre qui satisferait texte **et** silhouette
  (0,113 ≤ L ≤ 0,141) ne contient aucun token. Y entrer demande un token §2 neuf,
  décision §8 réservée à Vincent. À remonter avec les chiffres du § 3.3.
- **Élargir D6 à toute paire fond/texte des règles `.mika-*`** (§ 5.3) : population
  non mesurée, à ouvrir dans son propre vol avec sa propre Fire-Disposition.
- **Renommer `.mika-cta-gradient`** (§ 3.4) : *breaking change* d'API CSS, à
  grouper avec un bump majeur de la lib, jamais seul.
- **Les 13 primitives de `packages/ui/src` écrites sur des noms legacy** (§ 2.1,
  fait A). Dette de structure préexistante : c'est aujourd'hui la propriété qui
  fait marcher le § 2 — donc le geste n'est *pas* une migration vers le canon,
  mais une décision sur ce qu'une primitive doit servir à un consommateur neuf.
- **Publication npm, bump 0.6.0, rebase de mika-cloud#287** : gestes de Vincent,
  mika#2912 AC3.

Tracked in: senara-solutions/mika#2912

---

## 8. Fire-Disposition

Le plan livre sept détecteurs — D1 à D7 sont tous du code dont le chemin de succès
est « aucune violation trouvée ». Option retenue : **(a) exception nommée en
allowlist, dégénérée à zéro entrée.**

**Pourquoi zéro.** Les sept atterrissent **armés et verts**, sans exception, parce
que le même commit écrit la conformité qu'ils exigent : D1/D4 sur le bloc que
§ 4.1 réécrit, D5 sur les deux `index.css` que § 4.4 écrit, D2/D3 sur deux hex
vérifiés absents du fichier, D6/D7 sur le CTA que § 4.2 aplatit. Il n'existe
aucune violation préexistante **résiduelle** à exempter, donc aucune liste à
ouvrir. Inventer une allowlist vide ici serait du cérémonial :
`scripts/cta-primitives-allowlist.txt` ship vide parce que sa population est
**réelle** et pourrait croître ; la population de D1-D5 est un bloc de six lignes,
close par D4, et celle de D6-D7 est une règle CSS, close par son sélecteur.

**La violation préexistante de D6 est réelle, et elle est réparée, pas exemptée.**
C'est la différence qui compte ici : le CTA de 0.5.0 **viole** D6 aujourd'hui
(1,82:1), et AC9 exige qu'on le voie rougir avant de corriger (§ 5.3, rouge-avant).
Le vol ne l'allowliste pas, il le corrige dans le même commit. Un détecteur qui
atterrit vert après avoir été vu rouge sur la vraie donnée est la forme la plus
forte de (a) : la population est à zéro parce qu'elle a été assainie, pas parce
qu'elle n'a jamais été regardée.

**Où la substance de (a) s'applique quand même.** Les deux entrées *de contenu* de
D2 (`#1e2130`, `#ef4444`) sont des interdictions nommées qui survivent à leur
cause : chacune nomme la donnée précise, référence son ticket (mika#2913 § 2.2,
#1976), et **porte son assertion auto-nettoyante** — D3 les fait rougir le jour où
leur hex redevient canonique, c'est-à-dire le jour où l'interdiction cesse d'être
vraie. D4 joue le même rôle pour D1 : une table qui cesserait de décrire le bloc
rougit dans les deux sens. Aucune de ces entrées ne peut devenir stale en silence.

**Ce qui n'est pas désarmé, et pourquoi.** Aucun détecteur n'atterrit avec `.skip`
ni `#[ignore]` : l'option (b) existe pour un détecteur qui rougirait sur une
population qu'on ne peut pas assainir dans le vol, ce qui n'est pas le cas ici —
la seule population non assainissable (toute paire fond/texte des `.mika-*`) est
**hors du périmètre de D6 par décision écrite** (§ 5.3) et tracée au § 7, ce qui
est une borne et non un désarmement. Et aucun ne justifie (c) : le périmètre des
deux volets est écrit par Vincent, la règle §5 est rendue par Prime, et le § 3.2
ne laisse aucun choix de teinte à arbitrer.

---

## 9. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| **Volet A** — `packages/ui/src/theme.css` : bloc alias + commentaire du contrat | 16 |
| **Volet A** — `theme.test.ts` : D1 (table `legacyAliasValues` + assertion) | 16 |
| **Volet A** — `theme.test.ts` : D2 (`bannedLegacyHexValues` réduite à 2 entrées) | 6 |
| **Volet A** — `theme.test.ts` : D3 (anti-stale, les deux sens) | 22 |
| **Volet A** — `theme.test.ts` : D4 (exhaustivité du bloc, bidirectionnelle) | 18 |
| **Volet A** — `theme.test.ts` : D5 (les deux consommateurs re-aliasent) | 24 |
| **Volet A** — `dashboard/src/index.css` (bloc `@theme` + commentaire) | 12 |
| **Volet A** — `site/src/index.css` (bloc `@theme` + réécriture du commentaire l. 4-15) | 16 |
| **Volet B** — `packages/ui/src/theme.css` : CTA repos + hover + commentaire de tête | 22 |
| **Volet B** — `theme.test.ts` : helper WCAG + résolveur de couleur (3 formes, jette sinon) | 45 |
| **Volet B** — `theme.test.ts` : D6 (contraste de chaque arrêt, rest + hover, AC9) | 38 |
| **Volet B** — `theme.test.ts` : D7 (réécriture du describe « CTA texture ») | 20 |

Total estimé : 255 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient, et ce qui consomme la marge.** 255 contre 279 :
24 lignes, une marge mince, et c'est le volet B qui l'a mangée — un plan au volet A
seul pesait 182. Trois raisons de juger qu'elle suffit, et une césure si elle ne
suffit pas.

1. **Cinq fichiers, aucun nouveau, aucun Rust.** La boucle de vérification est
   `npm test -w packages/ui` (quelques secondes) plus deux builds Vite : il n'y a
   pas de `cargo build` dans le cycle, et c'est en tours que la borne est libellée.
   Les trois vols morts du 2026-10-14 étaient tous des vols de crate.
2. **Le poste le plus lourd est de la table.** `theme.test.ts` cumule 189 lignes,
   dont ~145 sont des tables et des `it.each` sur un fichier que le test lit déjà
   (l. 22). La seule logique neuve est le résolveur de 45 lignes — une fonction
   pure, vérifiable sans fixture, sans I/O et sans mock.
3. **Le budget commentaire est resserré, et c'est écrit dans le geste.** Le § 4.1
   demande ~10 lignes de commentaire là où un plan à un volet en prévoyait 22, et
   le § 4.4 en demande 6 par fichier. L'estimation ci-dessus les compte à ce
   budget, pas au budget d'un vol confortable.
4. **Aucune migration de callsite.** Les 791 lignes qui auraient fait exploser la
   borne sont renvoyées au § 7 avec `Tracked in:`, et le re-alias du § 4.4 est ce
   qui autorise ce renvoi sans laisser de fenêtre visuelle ouverte.

**Ordre de commit, et la césure.** Le chemin nominal est **une seule PR portant
les deux volets**, comme le périmètre l'écrit. Deux commits, dans cet ordre :

1. **Volet A** — § 4.1, § 4.4, D1-D5. Mergeable seul, et il porte `Closes #2913`.
2. **Volet B** — § 4.2, § 4.3, D6-D7, rouge-avant inclus. Il porte `Refs #2912`.

Cet ordre n'est pas neutre : il met d'abord au sec le défaut que le ticket porte.
**Si le compteur de tours dépasse ~110 avant que le commit 2 ne soit en place**,
le pilote ouvre la PR sur le volet A seul, écrit dans le corps que AC8-AC10
repartent au suivi, et les laisse sur `Tracked in: senara-solutions/mika#2912` —
qui reste ouvert pour AC3 de toute façon. C'est un repli sous contrainte de
plafond, pas un choix de périmètre : ce que la marge de 24 lignes ne garantit pas,
la césure le borne.

---

## 10. Risques

| risque | ampleur | mitigation |
|---|---|---|
| La marge de 24 lignes ne suffit pas | mort au plafond, le coût d'un pilote | Césure nommée au § 9 : volet A committé d'abord, repli documenté sur `Tracked in:`. |
| L'override `@theme` cross-`@import` ne prend pas | bloquant pour AC6/AC7 | Repli nommé au § 4.4 : les six redéfinitions dans `:root`. Vérifié sur le CSS buildé, pas supposé. |
| Le résolveur de couleur de D6 rend « indéterminé » sur une forme inconnue | D6 vert par ignorance — le pire défaut possible d'un détecteur | Il **jette** sur toute forme hors des trois admises, et un cas de test l'asserte (§ 5.3). |
| Les contrastes du § 3.2 sont calculés à la main dans ce plan | une erreur d'arithmétique déplacerait la décision de teinte | D6 est la source de vérité : il recalcule à partir de `theme.css` et rougirait sur un choix faux. Le rouge-avant/vert-après du § 5.3 le prouve sur la vraie donnée. |
| Un hex littéral glissé dans `site/src/index.css` ou dans une règle `.mika-` | `landing-tokens-lint` rouge ; `theme.test.ts` l. 504-511 rouge | Les deux s'écrivent en `var()` / `color-mix()` de tokens par construction (§ 3.2, § 4.4). |
| D5 lit deux fichiers hors de son package | chemins relatifs fragiles au déplacement | Même mécanisme que l. 22 (`readFileSync` relatif à `import.meta.url`) ; un déplacement casse le test au lieu de le rendre vacant. |
| Le CTA aplati collide avec le hover du bouton secondaire (même `secondary_container`) | grammaire visuelle : primary au repos = secondary au survol | Conséquence inévitable du § 3.2 (un seul token §2 passe le seuil). Nommée dans le corps de PR avec les chiffres du § 3.3 — c'est une information pour le mot de Vincent, pas un défaut à corriger ici. |
| Un test du dashboard ou de la lib assertant une classe legacy | `npm test` rouge | Aucune classe n'est renommée (§ 3.4) ; les assertions mesurées (`Button.test.tsx` l. 84/102/115, `ErrorState.test.tsx` l. 49/58, `Spinner.test.tsx` l. 41-42) portent sur la présence d'une classe ou une prop, jamais sur une valeur de couleur. |
| mika-cloud reçoit le legacy dans les primitives de la lib | voulu (§ 2.1), mais plus large que ce que le corps du ticket décrit | Nommé dans le corps de PR comme conséquence mesurée du fait A, avec la table du § 1.1. |

---

## 11. Definition of Done

- [ ] Les six alias de `theme.css` portent leur hex 0.3.1 littéral ; aucun `var(`
      dans le bloc (AC1).
- [ ] `--color-error` #ff6e84, `--color-surface-container-high` #1d2024 et
      `--color-secondary-container` #4434a0 inchangés ; `canonicalTokens` non
      modifiée (AC2).
- [ ] `bannedLegacyHexValues` conservée et réduite à `#1e2130` + `#ef4444`, chaque
      entrée nommant sa raison et son ticket (AC3).
- [ ] D1 et D4 livrés armés et verts ; une septième déclaration du bloc, ou un
      `var(`, fait rougir la suite, dans les deux sens (AC4).
- [ ] D3 livré : une entrée bannie devenue canonique fait rougir la suite, et
      symétriquement sur `legacyAliasValues` (AC5).
- [ ] D5 livré : les deux `index.css` re-aliasent les six noms, et le retrait de
      l'un fait rougir `npm test -w packages/ui` (AC6).
- [ ] No-op visuel du volet A vérifié sur le CSS buildé du dashboard et du site :
      les six noms résolvent vers les valeurs §2 (AC7).
- [ ] CTA aplati sur `var(--color-secondary-container)`, hover en `color-mix` vers
      `--color-background` ; aucun hex littéral dans la règle (AC8).
- [ ] D6 livré, **vu rouge sur 0.5.0 puis vert après correction**, les deux sorties
      recopiées dans le corps de PR ; une forme de couleur non résoluble fait
      rougir (AC9).
- [ ] D7 livré : le describe « CTA texture » réécrit sur la règle §5 amendée, pas
      supprimé ; le hover reste scopé hors `:disabled`.
- [ ] §5 de `luminescent-core.md` amendé avec la règle, sa provenance et son
      gardien ; §7 et §2 intacts (AC10).
- [ ] Suites vertes, lints inclus (AC11), et `scripts/verify-pipeline.sh origin/main`.
- [ ] `## Fire-Disposition` du § 8 reflétée dans le corps de PR.
- [ ] Corps de PR : `Closes #2913`, `Refs #2912` ; **dit que le merge attend le mot
      de Vincent sur l'amendement du rulebook** ; nomme le hors-vol du § 7 (npm,
      bump 0.6.0, rebase mika-cloud#287), le prix mesuré du § 3.3 (silhouette à
      2,07:1), la collision avec le hover secondaire, le fait A du § 1.1 et sa
      conséquence sur les primitives, et
      `Code review: delegated to mika-qa (mika#2694)`.
- [ ] `Tracked in: senara-solutions/mika#2912`.
