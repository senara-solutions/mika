# Plan — chore(ui,lc.5) : la landing consomme les tokens canon ; control-monitor est hors périmètre déclaré

**Status:** DRAFT
**Date:** 2026-09-20
**Ticket:** mika#1804
**Owner:** mika-dev (dispatch autonome)
**Class:** Design-system alignment (milestone Luminescent Core mika#1799, LC.5)
**Cross-refs:** mika#1800 (LC.1, `packages/ui/src/theme.css` ↔ rulebook §2), `docs/design/luminescent-core.md` §2/§3/§5, `docs/plans/2026-08-23-002-fix-1800-theme-css-rulebook-alignment-plan.md`

---

## Why

Le ticket décrit deux surfaces « off-brand » : la landing (`site/`) qui « forke un token subset », et `control-monitor` qui est « fully off-brand ». La lecture du code déplace le diagnostic sur les deux moitiés, et ces rectifications sont le premier livrable du plan — elles changent ce qu'il faut faire.

### R1 — Le paquet ne s'appelle pas `@senara-solutions/ui`

Le ticket écrit `@senara-solutions/ui`. Le paquet réel est **`@samidarko/ui`** (`packages/ui/package.json:2`), publié sur npmjs.org en accès public (mika#1386). Un plan qui reprend le nom du ticket produit un `npm install` qui échoue.

### R2 — La landing n'a pas forké : elle implémente une ligne du rulebook que LC.1 a supersedée

`site/src/index.css:6-11` déclare six tokens. Quatre d'entre eux sont **mot pour mot** la ligne « Override » du rulebook (`docs/design/luminescent-core.md:74`) :

> Override neutrals: `#0d0f12`. Override primary: `#7c6af7`. Override secondary: `#9d8fff`. Override tertiary: `#e8ecf2`.

Le rulebook §2 se contredit lui-même : sa table « Full Token Reference » (l.50-72) et sa ligne « Override » (l.74) donnent deux palettes. mika#1800 a tranché en faveur de la table **pour `packages/ui`** et l'a écrit dans le fichier (`packages/ui/src/theme.css:5-6`, `:25-26` — « Supersedes the "Override neutrals" override row », « Supersedes the "Override primary" row »). Cette décision n'a jamais été propagée à la landing.

Le drift n'est donc pas une dérive sauvage à réprimer, c'est **une non-propagation de LC.1**. Conséquence pratique : le travail est la suite directe de mika#1800, dont le plan avait explicitement déféré cette classe (`docs/plans/2026-08-23-002-…-plan.md:152` : « Aggressive rewrites of hand-rolled color usages … are **out of scope** — LC.2+ tickets in milestone #1799 address those component-by-component »).

Inventaire complet, mesuré :

| token `site` | valeur | provenance | canon §2 | verdict |
|---|---|---|---|---|
| `--font-sans` | Plus Jakarta Sans… | canon §3 | identique | ✅ **déjà aligné** |
| `--font-mono` | JetBrains Mono… | canon §3 | identique | ✅ **déjà aligné** |
| `--color-bg` | `#0d0f12` | Override neutrals (l.74) | `background` `#0c0e11` | drift (supersedé) |
| `--color-accent` | `#7c6af7` | Override primary (l.74) | `primary` `#ada3ff` | drift (supersedé) |
| `--color-accent-light` | `#9d8fff` | Override secondary (l.74) | `secondary` `#9d8fff` | ✅ identique |
| `--color-heading` | `#e8ecf2` | Override tertiary (l.74) | `on_surface` `#e8e8ec` | drift (supersedé) |
| `--color-bg-card` | `#151820` | aucune | `surface_container` `#171a1d` | fork local |
| `--color-muted` | `#a0a8b8` | aucune | `on_surface_variant` `#aaabaf` | fork local |

**La typographie est déjà conforme** — les deux familles sont identiques au canon §3 et les polices sont réellement chargées (`site/index.html:10`, Google Fonts). L'AC1 demande « min. palette + typography » : la moitié typographie est acquise, le plan la **verrouille** (garde-fou) plutôt que de feindre de la corriger.

### R3 — Le chemin d'alignement est déjà câblé, et le coût SEO redouté ne porte pas sur lui

Le ticket pose une décision binaire : « integrer `@senara-solutions/ui` vs stay landing-specific (poids/dépendances SEO) ». Elle est **séparable en deux**, et c'est ce qui débloque le ticket :

- **(a) consommer les _tokens_** — `@samidarko/ui` exporte `./theme.css` (`packages/ui/package.json:11`) qui pointe sur `src/theme.css` : du CSS pur, **zéro JavaScript, zéro composant React, aucun `dist` à builder**.
- **(b) consommer les _composants_** — React, `react-markdown`, `lucide-react` : le poids que le ticket craint à juste titre pour une landing.

On prend **(a) sans (b)**. Le coût en octets est celui de ~30 custom properties CSS.

Mieux : `packages/ui/src/theme.css:53-63` porte déjà six **alias de rétro-compatibilité** dont les noms sont **exactement** les six de la landing (`--color-bg`, `--color-bg-card`, `--color-accent`, `--color-accent-light`, `--color-heading`, `--color-muted`), avec ce commentaire : « Remove once a cross-repo grep sweep (cloud-console, **landing-page**) confirms zero legacy references. » La landing est nommément le consommateur que LC.1 attendait. Conséquence : **les ~70 classes utilitaires des 8 composants (`bg-accent`, `text-muted`, …) continuent de fonctionner sans être touchées**, mais résolvent vers le canon.

Et le patron de consommation existe en production — `dashboard/src/index.css` fait deux lignes :

```css
@import "tailwindcss";
@import '@samidarko/ui/theme.css';
```

avec `"@samidarko/ui": "*"` (`dashboard/package.json:14`), résolu par les npm workspaces de la racine, où `site` est **déjà** déclaré (`package.json:3-7`). Rien à inventer : on copie un consommateur qui marche.

### R4 — Le drift a trois canaux ; n'en corriger qu'un dégrade la page

Le ticket ne voit que le bloc `@theme`. Il y en a deux autres, et ils sont **mesurés** :

1. **`site/src/index.css:6-11`** — les six déclarations (le seul canal que le ticket voit).
2. **`rgba(124,106,247, …)` — la forme décimale de `#7c6af7` — en dur dans 6 composants** : `Hero.tsx:15` (halo radial), `HowItWorks.tsx:56` (séparateur en dégradé), `OpenSource.tsx:13` (3 valeurs), `Features.tsx:89` (ombre + inset), `Teams.tsx:114`, `Nav.tsx:36` (ombre du CTA).
3. **`site/index.html:12`** — `<body class="bg-[#0d0f12] text-[#a0a8b8] …">`, deux hex legacy hors token, sur le fond de page lui-même.

**Corriger le seul canal 1 produit une landing à deux violets et un fond de page désaccordé — un résultat pire que l'état actuel, qui est au moins cohérent.** Les canaux 2 et 3 ne sont donc pas des extras : ils sont la condition pour que l'alignement soit un progrès. C'est la classe de travail que mika#1800 avait déférée aux tickets LC.2+.

### R5 — Pour `control-monitor`, la décision demandée est déjà écrite dans le document normatif

Le ticket classe la question en « bearing question, potentiel escalade Prime ». Or le rulebook énonce son propre périmètre, en deuxième ligne (`docs/design/luminescent-core.md:4`) :

> **Scope:** Observability Dashboard, Cloud Console, Landing Page, and the shared `@samidarko/ui` component library.

Le périmètre est **énumératif**. La landing y est — ce qui fonde l'AC1 sur le document normatif lui-même, et non sur une préférence. `control-monitor` n'y est pas. La réponse à « cm doit-elle adopter Luminescent Core ? » n'est pas une décision à prendre, c'est une décision **déjà prise par le propriétaire du rulebook** qu'il suffit de constater et de citer.

Une nuance doit être posée honnêtement, parce qu'elle est la seule chose qui pourrait renverser la lecture : le silence du §Scope peut être une **exclusion** voulue ou une **omission** (cm a pu naître après la promotion du rulebook, 2026-04-25). Le plan produit donc la recommandation et sa rationale dans le dépôt, et **route la confirmation vers Vincent** — il ne tranche pas à sa place. C'est la différence entre une décision fondée et une décision fabriquée.

Fait vérifié au passage, puisque le ticket l'avance : `#3a82e0` est bien présent dans le bundle installé (`/usr/local/share/control-monitor/frontend/assets/index-*.js`). En revanche, `senara-solutions/control-monitor` **n'est pas dans ce workspace** — seul un artefact de build l'est. Aucune PR sur `mika` ne peut toucher ce code, ce que le ticket reconnaît (« Fix requiert ticket cross-repo OU décision »), et l'AC2 ne demande qu'une décision.

### R6 — Rien ne garde la landing aujourd'hui

`site` est un workspace npm mais **n'a aucun job CI** (`.github/workflows/ci.yml` ne connaît que `docs-site`, un autre répertoire). Aucun test, aucun lint de tokens. Tout garde-fou de non-régression est à créer : sans lui, le drift qu'on referme aujourd'hui se rouvre au premier `bg-[#7c6af7]` écrit de bonne foi.

---

## Requirements

1. La landing consomme les tokens canoniques depuis `@samidarko/ui/theme.css`, selon le patron du dashboard, sans dépendre d'un composant React de la bibliothèque.
2. Aucune valeur de la palette legacy ne subsiste dans `site/`, sur **aucun** des trois canaux.
3. Le résultat est visuellement cohérent : un seul violet, un fond de page accordé aux sections.
4. Un garde-fou exécutable refuse le retour de ces valeurs, et tourne en CI.
5. La conformité typographique déjà acquise est verrouillée, pas re-livrée.
6. Une décision écrite et datée existe pour `control-monitor`, avec sa rationale et son chemin de confirmation.
7. `docs/design/luminescent-core.md` n'est **pas** modifié (voir Hors périmètre).

---

## Design

### A — Landing

**Étape A1 — déclarer la dépendance.** `site/package.json` : ajouter `"@samidarko/ui": "*"` en `dependencies`. Résolu par les workspaces de la racine, sans publication ni build de `dist` — l'export `./theme.css` pointe sur les sources.

**Étape A2 — importer le canon.** `site/src/index.css` : remplacer le bloc `@theme` des six couleurs par l'import, exactement comme `dashboard/src/index.css` :

```css
@import "tailwindcss";
@import '@samidarko/ui/theme.css';
```

L'ordre importe (`tailwindcss` d'abord) ; c'est celui du dashboard. Les déclarations `--font-sans` / `--font-mono` du site deviennent redondantes (identiques au canon, apportées par `theme.css`) et disparaissent avec le bloc. **Ce qui est propre au site est conservé** : les keyframes `blink` / `type` et les classes `.cursor-blink` / `.type-reveal` (`index.css:18-40`), qui ne sont pas des tokens de marque.

Effet de bord à connaître : `theme.css` apporte aussi des styles globaux (scrollbar WebKit, `select option`, `html { scroll-behavior }`). Le `html { scroll-behavior: smooth }` du site devient redondant — même valeur. La scrollbar stylée est un changement visuel réel, mais aligné : c'est le comportement des autres surfaces Mika.

**Étape A3 — canal 2, les six `rgba` en dur.** Chaque occurrence doit **dériver du token** plutôt que de figer une nouvelle valeur — sans quoi on remplace un violet figé par un autre et le prochain mouvement du rulebook rouvre le même ticket. Forme recommandée :

- pour les utilitaires Tailwind : `bg-accent/12`, `shadow-accent/8` — la syntaxe d'opacité v4 s'applique aux couleurs de thème ;
- pour les `style={{ background: 'linear-gradient(…)' }}` : `color-mix(in srgb, var(--color-accent) 12%, transparent)`.

**Plancher navigateur — la condition est levée, pas laissée ouverte** (F2). `site/` ne déclare ni `browserslist` (`site/package.json`) ni `build.target` (`site/vite.config.ts`), donc deux planchers s'appliquent et c'est le plus haut qui lie :

| source | plancher |
|---|---|
| Vite 7 (`vite ^7.3.1`), défaut `baseline-widely-available` | Chrome 107, Edge 107, Firefox 104, Safari 16.0 |
| **Tailwind CSS v4** (`tailwindcss ^4.2.1`) — contraignant | **Chrome 111, Safari 16.4, Firefox 128** |
| `color-mix()` | Chrome 111, Edge 111, Safari 16.2, Firefox 113 |
| `rgb(from …)` (relative color syntax) | Chrome 119, Safari 16.4, Firefox 128 |

Le plancher liant est celui de Tailwind v4, et il est **au-dessus ou à égalité** du support de `color-mix()` sur les trois moteurs. Corollaire : `color-mix()` est disponible partout où la landing rend déjà quoi que ce soit — le plancher n'est pas un choix à faire, il est **déjà payé par une dépendance en place**, Tailwind v4 émettant lui-même du `color-mix()` pour ses modificateurs d'opacité (`bg-accent/12` en produit). Précédent en production dans ce dépôt : `dashboard/src/pages/SessionDetail.tsx:613`.

Conséquence sur la forme : **`color-mix()` est la forme retenue, sans chaîne de repli**, et `rgb(from …)` — dont le plancher Chrome est *plus haut* de 8 versions — n'est pas un repli mais une régression de compatibilité. Si un implémenteur veut malgré tout une custom property dédiée pour la lisibilité, c'est un choix de style sans effet sur le plancher.

Le critère d'acceptation reste la **propriété**, pas la forme : aucun littéral de couleur d'accent ne subsiste. Ce qui n'est en aucun cas acceptable, c'est de réécrire `rgba(173,163,255,…)` — on aurait remplacé un violet figé par un autre.

_Citation F2 : `docs/architecture/review-guide.md` § KISS — une précondition d'environnement non énoncée force l'implémenteur à sur-concevoir (forme la plus conservatrice par défaut) ou à sous-spécifier (choisir et espérer). Le tableau ci-dessus la pose une fois._

**Étape A4 — canal 3, le `<body>`.** `site/index.html:12` : `bg-[#0d0f12] text-[#a0a8b8]` → `bg-bg text-muted`. Ces classes existent déjà via les alias et résolvent vers le canon.

**Étape A5 — le garde-fou.** `scripts/check-landing-tokens.sh`, dans l'idiome du dépôt (`check-byte-slices.sh`, `check-dispatch-seats-declared.sh`, `check-a2a-timeout-literals.sh`) : refuse tout hex de la palette legacy dans `site/src/` et `site/index.html`, plus la forme décimale `124,106,247`. La liste des valeurs bannies est celle, déjà écrite et justifiée, de `packages/ui/src/theme.test.ts:102-110`.

Le script **doit** ignorer les commentaires (même raison qu'en `theme.test.ts:24-29` : une mention historique d'une valeur supersedée n'est pas une régression) et porter une **allowlist nommée** pour les trois pastilles de chrome de fenêtre — voir ci-dessous.

Job CI `landing-tokens-lint` dans `.github/workflows/ci.yml`, sur le modèle des trois lints existants. C'est la seule pièce qui empêche le drift de se rouvrir, et la seule qui donne à `site/` une vérification automatique quelconque.

**Drift intentionnel à documenter** (l'AC1 le prévoit explicitement) : `Hero.tsx:80-82` et `Teams.tsx:32-34` portent `#ff5f57` / `#febc2e` / `#28c840` — les pastilles de fenêtre macOS d'un mockup de terminal. Ce n'est pas de la palette de marque mais un skeuomorphisme référençant un chrome tiers ; les aligner sur le canon détruirait la citation visuelle. Elles restent, **nommées dans l'allowlist du script** avec cette raison, pour que l'exception soit une décision lisible et non un trou.

### B — control-monitor

**Étape B1.** Écrire `docs/design/control-monitor-scope-decision.md` : la recommandation (**rester distinct**), sa rationale citant `luminescent-core.md:4` et le caractère operator-tool de l'outil, la nuance exclusion-vs-omission, et le chemin de confirmation vers Vincent. Le fichier vit dans `docs/design/` à côté du rulebook — c'est déjà là que vit un document de réconciliation qui n'est pas le rulebook (`dashboard-stitch-map.md`).

**Aucun ticket cross-repo n'est ouvert** : l'AC2 ne le demande que « si adopte ». La recommandation étant « distinct », l'ouvrir serait agir contre sa propre conclusion. Si Vincent renverse la lecture, le ticket cross-repo est son geste, et le document le dit.

---

## Fire-Disposition

Ce plan livre un détecteur — `scripts/check-landing-tokens.sh` (A5) et son job CI `landing-tokens-lint`. La Fire-Disposition Gate (mika#1574) exige de nommer ce qui se passe quand il tire sur des données **existantes**, avant sa mise en service.

**Option canonique retenue : (a) Exception nommée en allowlist.**

**(i) Le tirage attendu.** Sans allowlist, le détecteur rouge dès sa première exécution sur du code que ce plan ne corrige pas : six littéraux hex, trois valeurs distinctes, deux sites.

**(ii) Les exceptions, énumérées.**

| hex | rôle | sites |
|---|---|---|
| `#ff5f57` | pastille « fermer » (chrome de fenêtre macOS) | `site/src/components/Hero.tsx:80`, `site/src/components/Teams.tsx:32` |
| `#febc2e` | pastille « réduire » | `site/src/components/Hero.tsx:81`, `site/src/components/Teams.tsx:33` |
| `#28c840` | pastille « agrandir » | `site/src/components/Hero.tsx:82`, `site/src/components/Teams.tsx:34` |

L'allowlist du script porte ces trois valeurs **avec leur raison écrite au même endroit**, pas dans un commit message : ce sont des couleurs de chrome de fenêtre tierce dans un mockup de terminal, pas de la palette de marque. Les aligner sur le canon détruirait la citation visuelle, qui est l'intention du composant.

**(iii) Tracker de suivi et assertion auto-nettoyante : écartés par conception, et c'est la partie qui doit être lue.** L'option (a) canonique les demande tous les deux. Ils sont refusés ici pour une raison de nature, pas de commodité :

- **Aucun tracker n'est ouvert** parce qu'il n'y a **rien à fermer**. Un tracker de suivi suppose un chemin de résolution — « ces valeurs partiront quand X ». Ici il n'y a pas de X : le skeuomorphisme est un état cible permanent. Un ticket ouvert sur une exception sans résolution est un ticket qui ne se ferme jamais, c'est-à-dire du bruit dans le registre avec l'apparence d'une dette.
- **Aucune assertion auto-nettoyante n'est posée** parce qu'il n'existe pas de condition d'obsolescence à asserter. Le patron auto-nettoyant (modèle : l'assertion `exportable` de mika#2292) rougit le jour où le fait qu'il garde cesse d'être vrai. Ici le fait gardé — « macOS dessine ses pastilles dans ces trois couleurs » — n'a pas de date de péremption contrôlée par ce dépôt. Une assertion qui ne peut jamais rougir n'est pas un garde-fou, c'est une ligne de test qui donne l'illusion d'en être un.

Ce qui remplace ces deux éléments, et qui est le vrai garde-fou : **l'allowlist est nominative par valeur, jamais par fichier ni par motif**. Un futur `#ff5f57` écrit ailleurs qu'en pastille passerait — limite nommée, acceptée, et bornée par le fait que ces trois valeurs ne sont pas des couleurs de marque plausibles. Un futur `bg-[#7c6af7]` écrit de bonne foi, lui, rougit. Exclure les deux **fichiers** (`Hero.tsx`, `Teams.tsx`) de la vérification aurait ouvert un trou réel : ce sont deux des six composants portant du `rgba(124,106,247,…)` au canal 2.

**(iv) Zéro autre violation pré-existante — vérifié, pas supposé.** L'inventaire exhaustif des littéraux hex de `site/` (`grep -rnoiE "#[0-9a-f]{6}" src/ index.html`, 2026-09-20) rend **quatorze** occurrences et rien d'autre :

- huit sont des valeurs de la palette legacy (`index.css:6-11` × 6, `index.html:12` × 2) — **toutes supprimées par A2 et A4** ;
- six sont les pastilles ci-dessus.

Le canal 2 (la forme décimale `124,106,247` dans six composants) est **entièrement corrigé par A3**. L'allowlist est donc l'ensemble **complet** des exceptions, et non un premier lot : après ce plan, le détecteur tire exactement sur zéro ligne de `site/`. C'est ce qui rend V5/V6 (les contrôles négatifs) significatifs — un lint qui part déjà rouge ne prouve rien quand on le voit rougir.

_Citation : `mika-arch-groom-ticket` § Fire-Disposition Gate (mika#1574), branche 2 de l'arbre de décision._

---

## Verification contract

| # | Vérification | Commande / geste | Attendu |
|---|---|---|---|
| V1 | Résolution de la dépendance | `npm install` à la racine | `site/node_modules/@samidarko/ui` est un lien de workspace |
| V2 | Build de la landing | `npm run build --workspace=site` | exit 0, aucune erreur de résolution du `@import` |
| V3 | Palette canon servie | inspecter le CSS produit sous `site/dist/assets/` | `#0c0e11`, `#ada3ff`, `#171a1d`, `#aaabaf`, `#e8e8ec` présents |
| V4 | Zéro legacy | `scripts/check-landing-tokens.sh` | exit 0 |
| V5 | **Contrôle négatif** | réintroduire `bg-[#7c6af7]` dans un composant, relancer V4 | exit ≠ 0, le fichier et la ligne sont nommés ; **puis révoquer** |
| V6 | Contrôle négatif (canal 2) | réintroduire `rgba(124,106,247,0.1)`, relancer V4 | exit ≠ 0 |
| V7 | Typographie verrouillée | V4 couvre l'absence de régression ; vérifier que `Plus Jakarta Sans` / `JetBrains Mono` sont toujours servis (`site/index.html:10` intact) | familles inchangées |
| V8 | CI | job `landing-tokens-lint` sur la PR | vert, et rouge sur le commit de contrôle négatif si on le pousse |
| V9 | Cohérence visuelle | `npm run preview --workspace=site`, parcourir les 7 sections | un seul violet ; fond de page et cartes accordés ; halos et ombres suivent l'accent canon |
| V10 | Non-régression du dashboard | `npm test --prefix packages/ui` | vert — `theme.css` n'est pas modifié par ce travail |

V5 et V6 sont les vérifications qui comptent : un lint qu'on n'a pas vu rougir n'est pas un lint, c'est une ligne de CI.

---

## Definition of Done

- `site/` ne porte plus aucune valeur de la palette legacy, sur les trois canaux.
- La landing rend la palette canon, vérifié dans le CSS produit **et** à l'œil (V9).
- `scripts/check-landing-tokens.sh` existe, est branché en CI, et a été vu rougir (V5/V6).
- L'allowlist du script nomme les trois pastilles macOS **par valeur** et dit pourquoi, conformément au § Fire-Disposition (option (a), exception nommée).
- `docs/design/control-monitor-scope-decision.md` existe, avec recommandation, rationale citée et chemin de confirmation.
- `docs/design/luminescent-core.md` est inchangé (`git diff` vide sur ce fichier).
- `packages/ui/src/theme.css` est inchangé.
- Le corps de PR nomme : le drift intentionnel des pastilles, le risque de contraste ci-dessous, et les deux suivis identifiés.

## Acceptance criteria

Transcrits du corps de mika#1804 :

1. **Landing : tokens alignés canon (min. palette + typography), drift documented as intentional si applicable.**
   - Palette : étapes A1–A4, vérifiée par V3/V4/V9.
   - Typography : **déjà conforme** avant ce travail (R2) ; verrouillée par V4/V7.
   - Drift intentionnel : les pastilles macOS, documentées dans l'allowlist du script et dans le corps de PR.
2. **control-monitor : décision explicite « adopte » vs « distinct » + rationale. Si adopte, ticket cross-repo filé `senara-solutions/control-monitor`.**
   - Décision « distinct » recommandée, rationale citant `luminescent-core.md:4`, dans `docs/design/control-monitor-scope-decision.md` (étape B1).
   - La branche conditionnelle n'est pas déclenchée : la recommandation étant « distinct », aucun ticket cross-repo n'est ouvert. Le document nomme le geste à faire si Vincent renverse.

---

## Out of scope

- **Modifier `docs/design/luminescent-core.md`.** Il est Vincent-owned, en commit direct, et « not relitigated through PRs » (`luminescent-core.md:5`). Deux corrections y seraient pourtant utiles — supprimer ou dater la ligne « Override » du §2 (l.74) que LC.1 a supersedée et que ce ticket supersède une seconde fois, et nommer explicitement `control-monitor` comme hors périmètre. **Ce sont des gestes d'opérateur**, signalés dans le corps de PR, pas des lignes de ce diff.
- **Renommer les classes de la landing vers les noms canoniques** (`bg-accent` → `bg-primary`, `text-muted` → `text-on-surface-variant`, …). ~70 occurrences dans 8 composants, **strictement mécanique et sans aucun effet visuel**. Écarté délibérément : le mélanger noierait, dans un diff de renommage, le changement visuel qui mérite une vraie review. Coût nommé : tant qu'il n'est pas fait, la landing reste consommatrice des alias de rétro-compat, et le sweep de suppression annoncé par `packages/ui/src/theme.css:57` reste bloqué. **Ticket de suivi.**
- **Le contraste du CTA.** `Nav.tsx:36` rend `bg-accent text-white`. Sur l'accent canon `#ada3ff`, le contraste blanc tombe à **≈2,24:1** — échec WCAG AA franc (4,5:1 requis). Mais ce n'est **pas une régression introduite ici** : `dashboard/src/pages/Timeline.tsx:94`, `Traces.tsx:41` et `packages/ui/src/components/ErrorState.tsx:46` portent déjà `text-white` sur ce même accent. Aligner la landing **propage** un défaut existant vers une surface customer-facing. Inventer un remède local serait créer un nouveau drift landing-vs-dashboard, c'est-à-dire refaire le défaut qu'on referme. **Mesuré, nommé, routé en ticket de suivi** couvrant les quatre sites d'un coup.
- **Le CTA en dégradé.** Le rulebook §5 exige pour un bouton primaire un dégradé `primary` → `primary_dim` à 135° et un arrondi `xl` ; la landing rend un aplat `rounded-full`. C'est §5 Components, pas §2 palette ni §3 typography — hors du minimum de l'AC1. À traiter avec le contraste, dont il est la moitié du remède (`primary_dim` `#715eeb` remonte le contraste blanc à ≈4,64:1).
- **Le code de `control-monitor`.** Hors dépôt, hors workspace, hors PR.
- **La meta description de la landing.** `site/index.html:7` affirme « runs entirely on your machine ». C'est la classe de revendication d'hébergement que mika#2290 a dû fermer côté agent, ici sur une surface customer-facing. Sans rapport avec les tokens ; **signalé pour ticket**, délibérément pas traité ici.

## Risques

| Risque | Probabilité | Mitigation |
|---|---|---|
| `@import '@samidarko/ui/theme.css'` ne résout pas au build | faible | Patron identique au dashboard, en production. V1/V2 le prouvent avant toute autre étape. |
| Régression visuelle non vue par le lint (le lint voit des valeurs, pas un rendu) | moyenne | V9 est manuelle et obligatoire ; le diff est petit et lisible. |
| Les alias de compat sont supprimés plus tard et cassent la landing | faible | Le commentaire de `theme.css:53-57` conditionne leur suppression à un sweep qui constaterait zéro référence : la landing en restant consommatrice **empêche** la suppression silencieuse. Le suivi de renommage ci-dessus est la sortie propre. |
| La scrollbar stylée apportée par `theme.css` surprend en review | faible | Nommé ici et dans le corps de PR ; c'est un alignement, pas un accident. |
| La recommandation B est renversée par Vincent | réelle et acceptée | Le document est une recommandation datée avec son chemin de confirmation, pas un fait accompli. Le renversement coûte un ticket cross-repo, que le document nomme déjà. |

---

## Revision history

- **rev 2 (2026-09-20)** — révision en réponse aux findings de la première passe architecte (`.iterate/findings-1.md`).
  - **F1 (BLOCKING) adressé** : ajout d'une section `## Fire-Disposition` de plein droit, là où le contenu d'allowlist n'était que de la prose dispersée en Design §A5. Elle (i) nomme l'option canonique **(a) Exception nommée en allowlist**, (ii) énumère les trois hex de pastille macOS avec leurs six sites, (iii) **écarte explicitement le tracker de suivi et l'assertion auto-nettoyante, avec leur raison de nature** — pas de chemin de résolution donc rien à fermer ; pas de condition d'obsolescence donc rien à asserter — et nomme ce qui les remplace (allowlist nominative **par valeur**, jamais par fichier, avec sa limite acceptée), et (iv) confirme par inventaire exhaustif daté que les quatorze littéraux hex de `site/` se partagent entre huit valeurs legacy supprimées par A2/A4 et les six pastilles, soit **zéro autre violation pré-existante** — l'allowlist est l'ensemble complet, pas un premier lot. Conséquence portée sur la DoD : l'allowlist est nominative par valeur. Citation préservée : mika#1574, branche 2.
  - **F2 (sharpening) adressé** : A3 ne pose plus de conditionnel non évaluable. Le plancher navigateur de `site/` est établi (ni `browserslist` ni `build.target` déclarés ⇒ deux planchers, le liant étant Tailwind v4 : Chrome 111 / Safari 16.4 / Firefox 128) et **`color-mix()` est disponible partout où la landing rend déjà**, Tailwind v4 en émettant lui-même. La chaîne de repli disparaît : `color-mix()` devient la forme retenue et `rgb(from …)`, dont le plancher Chrome est plus haut de 8 versions, est requalifié de régression de compatibilité plutôt que de repli. Précédent en production cité (`dashboard/src/pages/SessionDetail.tsx:613`). Citation préservée : `review-guide.md` § KISS.
  - Aucun AC affaibli ; aucune autre section réécrite.
