# mika#1799 — vol 1 : le détecteur CTA ne regarde pas le paquet qui définit `<Button>`, et ne voit pas un gradient comme un fill

- **Ticket** : senara-solutions/mika#1799 — *milestone: Luminescent Core reconciliation — code aligné au rulebook*
- **Type** : `fix(ui,lc.7)`
- **Suite** : vol 2 — `Tracked in: senara-solutions/mika#1799` (le milestone reste ouvert, § 7)

---

## 1. Contexte : le milestone a livré ses six axes, et un défaut a survécu entre deux d'entre eux

Les sept sous-tickets de #1799 sont fermés :

| | ticket | état |
|---|---|---|
| LC.1 | mika#1800 — `theme.css` → rulebook §2 | CLOSED (`1033424c`) |
| LC.2 | mika#1801 — extraction `<Button>`, `<Spinner>` + armement du détecteur CTA | CLOSED (`fab69e13`) |
| LC.2b | mika#2562 — `<SecretField>`, `<AuthCard>`, `<Stepper>` | CLOSED (`a27e29aa`) |
| LC.3 | mika#1802 — adoption customer-facing | CLOSED |
| LC.4 | mika#1803 — skew de registre Console | CLOSED |
| LC.5 | mika#1804 — surfaces off-brand | CLOSED |
| LC.6 | mika#1805 — profondeur de test des primitives | CLOSED |

Le milestone reste pourtant ouvert, et il a raison de l'être : **une violation du rulebook §5 subsiste, dans le paquet même qui définit la primitive censée l'empêcher**, et le détecteur armé par LC.2 ne peut pas la voir — pour deux raisons indépendantes qui se recouvrent exactement sur elle.

### 1.1 La violation

`packages/ui/src/components/ErrorState.tsx:46` — le bouton « Retry » de l'état d'erreur de liste :

```tsx
className="px-4 py-1.5 text-xs font-medium rounded-lg bg-gradient-to-r from-accent to-accent-light text-white hover:opacity-90 transition-opacity"
```

Quatre écarts, mesurés contre le rulebook :

| | ce qui est écrit | ce que dit le rulebook |
|---|---|---|
| angle | `bg-gradient-to-r` = 90° | §2 « Signature Textures » : **135°** |
| arrivée | `to-accent-light` = `--color-secondary` (`#9d8fff`) | §2 : `primary` → **`primary_dim`** (`#715eeb`) |
| encre | `text-white` | §7 Don't — « **No Pure White**: Never use `#ffffff` » |
| primitive | `<button>` à la main | §5.5 l.272, qui parle de **ce bouton-ci** : « `retry` … — **primary gradient button per rulebook §5** » |

Et un cinquième, qui est le plus sévère : par le défaut mesuré au § 7 (Tailwind ne scanne pas ce paquet), **aucune de ces classes n'est générée**. Le bouton ne rend probablement aucun fill du tout. Le rulebook nomme ce bouton comme le lieu de sa signature visuelle, et c'est le seul bouton du dépôt qui n'en a pas.

C'est la dernière CTA hand-rolled des trois surfaces. Mesure, sur `dashboard/src`, `site/src` et `packages/ui/src` :

```
$ grep -rn --include='*.tsx' --include='*.jsx' 'bg-gradient\|from-accent\|to-accent' dashboard/src site/src packages/ui/src
packages/ui/src/components/ErrorState.tsx:46: …bg-gradient-to-r from-accent to-accent-light text-white…
```

**Une occurrence. Population connue : 1.**

### 1.2 Pourquoi le détecteur ne l'a jamais vue — D1, le périmètre

`scripts/check-cta-primitives.sh:87` :

```bash
SCAN_DIRS="dashboard/src site/src"
```

LC.2 a armé le détecteur sur les deux surfaces *consommatrices*, et pas sur le paquet *producteur*. C'est le choix naturel au moment où l'on migre dix CTA de consommateurs — et c'est aussi le seul endroit où la primitive pouvait se contredire elle-même sans que rien ne le dise.

### 1.3 Pourquoi le périmètre n'aurait pas suffi — D2, le faux négatif du prédicat

`check-cta-primitives.sh:229` range le gradient parmi les utilitaires **non-couleur** :

```awk
if (tok ~ /^bg-(clip|origin|blend|gradient|linear|radial|conic)-/) { continue }
```

L'intention est juste : `bg-clip-text`, `bg-origin-border`, `bg-blend-multiply` ne peignent rien. Mais `bg-gradient-to-r` **accompagné de stops** (`from-*` / `to-*`) peint le fill le plus CTA qui soit — c'est littéralement la texture que §2 prescrit pour une primary. L'exclusion est née d'un état fondateur qui le dit lui-même (`check-cta-primitives.sh:12`) : *« Ten CTAs … none of them a gradient »*. Le cas n'existait pas, donc il est tombé dans le paquet voisin.

**Mesuré, pas supposé.** Sonde : `packages/ui/src` copié sous `.pilot-scratch/probe/dashboard/src`, détecteur courant lancé dessus —

```
$ bash scripts/check-cta-primitives.sh .pilot-scratch/probe
dashboard/src/components/LiveRefreshToggle.tsx:23: hand-rolled CTA — <button> painting bg-accent
Found 1 hand-rolled CTA fill(s).
```

`ErrorState.tsx:46` **n'y est pas**. Étendre `SCAN_DIRS` seul aurait laissé la violation passer — et le PR aurait porté « périmètre étendu, zéro violation », ce qui est la lecture exactement fausse.

Sonde ré-exécutée telle quelle au re-grooming du 2026-10-19, sortie identique à la ligne près. C'est le seul fait de ce plan qui ne se lit pas dans un fichier, donc le seul qui méritait d'être revu plutôt que relu.

**Et un troisième fait, que la même sonde donne par son silence** : `Button.tsx` — le producteur, 5 call-sites `<Button>` dans sa propre JSDoc, et un `<button>` réel à la ligne 128 — **ne fait pas feu**. Il peint par `.mika-btn--*` / `.mika-cta-gradient`, qui sont du CSS réel dans `theme.css` et non des utilitaires `bg-*`. L'objection naturelle à D1 — « étendre le périmètre au paquet producteur va faire rougir le producteur » — est donc déjà réfutée par la mesure, avant écriture.

### 1.4 Et ce que l'extension aurait accusé à tort — D3, le faux positif

La même sonde fait feu sur `packages/ui/src/components/LiveRefreshToggle.tsx:23` :

```tsx
<button role="switch" aria-checked={isLive}
  className={`relative w-9 h-5 rounded-full … ${isLive ? 'bg-accent' : 'bg-white/[0.1]'}`}>
```

Ce n'est pas une CTA : c'est le *track* d'un interrupteur, et `<LiveRefreshToggle>` est déclaré primitive canonique dans `packages/ui/CLAUDE.md`. Le prédicat « fill solide sur un `<button>` » ne peut pas l'en distinguer, parce que la distinction n'est pas dans le fond — elle est dans le **rôle** : un élément qui déclare `role="switch"` porte un **état**, pas une **action**.

L'en-tête du détecteur dit pourquoi ce cas doit être fermé par le prédicat et jamais par l'allowlist : *« un faux positif ici coûte une PR bloquée et se règle en mettant le lint en sourdine, ce qui est le mode de défaillance à fermer en premier. »*

Mesure de la population de rôles d'état sur les trois surfaces : **une occurrence**, celle-ci.

### 1.5 Ce que ce vol corrige, en une phrase

Le mécanisme central du milestone — *hand-rolled → primitive, tenu par une machine* — n'était appliqué ni au paquet qui porte la primitive, ni à la texture que le rulebook désigne comme sa signature. Ce vol ferme les deux, et migre l'unique violation que leur conjonction cachait.

---

## 2. Les livrables, site par site

### D1 — `scripts/check-cta-primitives.sh` : le périmètre

```bash
SCAN_DIRS="dashboard/src site/src packages/ui/src"
```

Avec, dans l'en-tête, la raison écrite : *le paquet producteur est la seule surface où la primitive peut se contredire elle-même, et c'est là qu'elle l'a fait.*

**Les fichiers de test et d'assertion de types sortent du périmètre** — `*.test.tsx`, `*.test.jsx`, `*.type-assertions.tsx` — par un filtre sur le `find` de la construction de `FILES`. Deux raisons, dans cet ordre :

1. Un fixture de test n'est pas une surface rendue. Le scan existe pour tenir ce que l'utilisateur voit.
2. Sans le filtre, le compte d'anti-vacuité passe de 10 à **75** et **devient instable**. La décomposition, mesurée par fichier le 2026-10-19 :

   | | call-sites `<Button>` |
   |---|---|
   | `Button.test.tsx` | 26 |
   | `Button.type-assertions.tsx` | 25 |
   | `Spinner.test.tsx`, `SecretField.test.tsx` | 1 + 1 |
   | **sous-total fichiers de test** | **53** |
   | `Button.tsx` (5, JSDoc comprise), `SecretField.tsx` (3), `Spinner.tsx` (2), `Stepper.tsx` (1), `AuthCard.tsx` (1) | **12** |

   Les **53** sont ce que le filtre retire. L'assertion `assert_says "… CTA element(s)"` casserait sinon à chaque test ajouté à `<Button>` — un harness qui rougit sur du travail sain est un harness qu'on désarme.

   Les **12** restants sont la part réelle de l'extension, et ils donnent l'ordre de grandeur attendu du nouveau compte : **au plus 22** (10 + 12), « au plus » parce que le détecteur dédupe un `<Button>` nommé dans un commentaire (harness l.203) et que 5 des 12 sont dans la JSDoc de `Button.tsx`. Ce nombre est une **borne d'orientation pour l'implémentation, pas une valeur à recopier dans l'assertion** : celle-ci prend ce que le scan filtré rend sur l'arbre du jour (§ 4.2).

Le filtre est **neutre sur l'existant**, et c'est vérifié : `dashboard/src` contient déjà 5 fichiers `.test.tsx` dans le périmètre courant, et leur population CTA est nulle (`grep -c '<Button\|<button\|<a' ` → `0` sur les cinq). Le compte reste 10 après le filtre, avant extension.

Le message d'anti-vacuité (`check-cta-primitives.sh:307`) nomme aujourd'hui « dashboard/src and site/src » et le nombre dix ; il est réécrit pour nommer les trois répertoires.

### D2 — `check-cta-primitives.sh` : un gradient avec stops est un fill

L'exclusion `bg-(clip|origin|blend|gradient|linear|radial|conic)-` est **conservée** pour ce qu'elle visait, et perd le seul cas qui peint :

- un token `bg-gradient-to-*` ou `bg-linear-*` (Tailwind v4 renomme le premier en le second ; les deux orthographes coexistent dans la nature) **cesse d'être exclu dès que le même tag porte un stop couleur** — un `from-<token>`, avec ou sans `via-` / `to-`.
- Le token rapporté dans le message de violation est le **stop** (`from-accent`), pas le mot `bg-gradient-to-r` : c'est le stop qui nomme la couleur qui a cessé de suivre §2, et c'est la chaîne qu'un lecteur va chercher dans le fichier.
- Les trois exclusions existantes restent dominantes et inchangées : un `/` dans le stop (`from-accent/40`) reste translucide donc hors population ; un préfixe de variante (`hover:from-accent`) reste hors population ; `bg-clip-text` seul reste hors population.

**Risque de faux positif mesuré avant écriture** : le motif « texte en dégradé » (`bg-clip-text text-transparent` + `from-*`) est fréquent sur une landing et serait le faux positif naturel de cette règle. Mesure sur les trois surfaces :

```
$ grep -rn --include='*.tsx' --include='*.jsx' 'bg-clip-text\|text-transparent\|from-\|via-\|to-\[' dashboard/src site/src packages/ui/src
packages/ui/src/components/ErrorState.tsx:46
```

Aucune occurrence hors de la violation elle-même. La règle n'a, sur cet arbre, **aucun autre positif que celui qu'elle existe pour attraper** — et le cas `bg-clip-text` est néanmoins couvert par un fixture vert (N7, § 4), parce qu'un scan dont on n'a pas vu la retenue n'est pas distinguable d'un scan qui fire sur tout.

### D3 — `check-cta-primitives.sh` : un rôle d'état n'est pas une action

Quatrième exclusion, écrite en **forme** comme les trois autres : un tag qui déclare `role="switch"`, `role="checkbox"`, `role="radio"`, `role="tab"`, `role="menuitemcheckbox"` ou `role="menuitemradio"` sort de la population, quel que soit son fill.

La ligne de partage est explicite dans l'en-tête : ces rôles ARIA portent un **état sélectionné** (`aria-checked`, `aria-selected`), et un contrôle d'état peint son fond pour *dire dans quel état il est*, non pour appeler à l'action. `role="button"` et `role="link"` ne sont évidemment pas de la liste — ce sont des CTA qui se sont donné un rôle explicite, et le scan doit continuer de les voir.

Écrit comme exclusion de forme et non comme `# LiveRefreshToggle exception` : la régression réaliste n'est pas quelqu'un qui réécrit ce toggle, c'est quelqu'un qui écrit le prochain `role="tab"` avec un onglet actif en `bg-accent`.

### D4 — `packages/ui/src/components/ErrorState.tsx` : la migration

| site | avant | après |
|---|---|---|
| l.43-49, variante `list` | `<button>` + gradient 90° + `text-white` | `<Button variant="primary" size="sm" onClick={retry}>Retry</Button>` |
| l.52-59, variante `list` | `<a target="_blank" rel="noopener noreferrer" className="text-accent …">` | `<Button as="link" href={detailsHref} external variant="tertiary" size="sm">` |
| l.22-28, variante `detail-section` | `<button className="text-accent text-xs …">Retry</button>` | `<Button variant="tertiary" size="sm" onClick={retry}>Retry</Button>` |

Les deux dernières lignes sortent du périmètre du détecteur (pas de fill ⇒ pas de CTA au sens du scan) et sont migrées **par réemploi, pas par contrainte** — en le disant dans le corps de PR plutôt qu'en le faisant passer pour une exigence du lint :

- le lien détails réimplémente exactement ce que `external` produit (`target="_blank" rel="noopener noreferrer"`, `Button.tsx:135`) ;
- les trois souffrent du même non-rendu (`text-accent`, `hover:text-accent-light` ne sont pas générées pour ce paquet, § 7), que `.mika-btn--tertiary` répare puisqu'il est du CSS réel dans `theme.css` ;
- §5.5 l.238 décrit la variante `detail-section` comme « compact inline, "Retry" link only » : `variant="tertiary" size="sm"` est la traduction littérale de « text-only, low-priority » de §5.

**Ce que la migration change à l'œil, nommé plutôt que découvert en revue** : `size="sm"` apporte `padding: spacing-2 spacing-3` et `border-radius: var(--radius-mika-xl)` là où le Retry de liste écrivait `px-4 py-1.5 rounded-lg`. Le rayon passe de `lg` à `xl`, ce que §5 prescrit pour une primary (« `xl` (1.5rem) roundedness ») et §7 pour tout élément interactif. C'est le but, pas un effet de bord.

### D5 — `packages/ui/src/theme.css` et `theme.test.ts` : deux commentaires qui décrivent un monde révolu

LC.1 a remonté à Vincent la contradiction §2 (`#ff6e84`) / §5.5 (`#ef4444`) plutôt que de la trancher, le rulebook étant Vincent-owned. **Elle a été tranchée** — `76a8b0de`, PR #1976, « reconcile rulebook §5.5 error hex → #ff6e84 ». `luminescent-core.md:270` lit aujourd'hui `#ff6e84`.

Trois commentaires affirment encore qu'elle est vivante :

| site | état |
|---|---|
| `theme.css:40-43` | « §5.5 line 270 references #ef4444 (contradiction with §2's #ff6e84) … surfaced to Vincent as operator follow-up » |
| `theme.test.ts:82-83` | « §5.5 line 270 contradiction surfaced to Vincent … » |
| `theme.test.ts:84` | libellé `'error (§2 canonical, adopted over §5.5 #ef4444)'` |

Les trois deviennent le constat de résolution, avec son commit. **L'entrée `['#ef4444', …]` de `bannedLegacyHexValues` (`theme.test.ts:109`) est conservée** : la contradiction est résolue, mais la valeur reste une valeur dont la réintroduction serait une régression — seul son libellé change pour nommer #1976. C'est une distinction qui mérite d'être écrite : un commentaire périmé se corrige, une garde ne se retire pas parce que la cause de sa naissance a été réglée.

### D6 — `packages/ui/CLAUDE.md` (hors compte de lignes)

La règle « Hand-rolled CTA fills are forbidden » énonce son périmètre comme « sous `dashboard/src` ou `site/src` ». Il devient les trois répertoires, avec la mention de l'exclusion des fichiers de test et des deux cas de prédicat ajoutés. La table des primitives gagne la mention que `<ErrorState>` rend désormais ses actions via `<Button>`.

---

## Fire-Disposition

Ce vol livre un détecteur (il étend `scripts/check-cta-primitives.sh`, dont le chemin de succès est « aucune violation trouvée »). Disposition retenue : **(a) exception nommée en allowlist — avec un ensemble d'exceptions vide, parce que la population entière est migrée dans le même diff.**

Le détail d'implémentation, qui est ce qui rend (a) non-vacue ici :

- **La population est mesurée, pas estimée.** Les trois extensions ont chacune une population connue sur l'arbre au 2026-10-19 :
  - D1+D2 (périmètre + gradient) → **1** : `ErrorState.tsx:46`, migrée par D4 dans le même commit.
  - D3 (rôle d'état) → **1** faux positif supprimé : `LiveRefreshToggle.tsx:23`, fermé par le prédicat.
  - D1 (fichiers de test) → **0** sur l'existant (vérifié sur les 5 `.test.tsx` de `dashboard/src`).
- **`scripts/cta-primitives-allowlist.txt` reste vide, et l'assertion qui l'y tient n'est pas touchée.** Le harness pin la vacuité de l'allowlist ; `packages/ui/CLAUDE.md` énonce la conséquence : *« adding the first entry means deleting the assertion first, in the same diff. »* Ce vol **n'ajoute aucune entrée et ne supprime aucune assertion**. Le diff ne touche pas `cta-primitives-allowlist.txt`.
- **La moitié auto-nettoyante existe déjà et continue de couvrir ce vol** : une entrée d'allowlist qui ne correspond à aucune violation réelle fait rougir le build (`check-cta-primitives.sh`, section « The self-cleaning half »). Une exception future née de cette extension ne pourra pas survivre à sa cause.
- **La retenue du détecteur est vue verte, pas supposée.** Les cas N6 (`role="switch"` + `bg-accent` → vert), N7 (`bg-clip-text` seul → vert) et N9 (`hover:from-accent`, `from-accent/40` → vert) sont dans le harness pour la raison que son propre en-tête donne : sans eux, « le scan attrape les gradients de CTA » est indistinguable de « le scan rougit sur tout `from-` », et la seconde forme serait mise en sourdine dans la semaine.
- **La condition d'arrêt est écrite** : si l'implémentation découvre, en lançant le détecteur étendu sur l'arbre réel, une population **supérieure à celle mesurée ici** (une violation autre qu'`ErrorState.tsx:46`, ou un faux positif autre que `LiveRefreshToggle.tsx:23`), elle **s'arrête et remonte à l'opérateur** — bascule explicite vers (c) — plutôt que d'allouer le budget du vol à une migration non cadrée ou d'ouvrir l'allowlist. Les deux nombres ci-dessus sont la mesure contre laquelle l'arrêt se déclenche.

**Pourquoi pas (b), livrer désarmé.** C'est l'option qui existe quand la population est trop grande pour tenir dans le vol, et il faut donc dire pourquoi elle ne s'applique pas ici : la population est de **un**, et elle est migrée dans le même diff. Un détecteur livré sous `#[ignore]` pour une violation unique déjà corrigée protégerait exactement zéro ligne de plus que le détecteur armé, tout en ajoutant un suivi d'armement — c'est-à-dire un ticket pour rallumer une garde qui n'avait aucune raison d'être éteinte. Et le coût est connu de ce dépôt : une garde non déployée se lit comme une flotte en bonne santé, ce que l'en-tête du détecteur nomme déjà par ticket (`check-cta-primitives.sh:300`, mika#2205). (b) serait le choix si l'arrêt du point précédent se déclenchait et que l'opérateur tranchait pour une migration différée — ce n'est alors plus ce plan.

---

## 3. Ce que le vol ne fait PAS

- **Il ne touche pas `luminescent-core.md`.** Le rulebook est Vincent-owned, par commit direct, et n'est pas relitigé en PR (`CLAUDE.md § docs/design`). D5 aligne le *code* sur une décision déjà prise dans le rulebook ; il ne décide rien.
- **Il n'ajoute aucune variante à `<Button>`.** Les trois variantes de §5 suffisent aux trois sites migrés. Une variante nouvelle serait une décision §8 réservée à Vincent.
- **Il ne migre pas les boutons non-CTA de `packages/ui`.** `CopyButton`, `Pagination` (×2), `TimeRangeFilter` (×2), l'action de `EmptyState`, le track de `LiveRefreshToggle` : ce sont des icon buttons, de la navigation, des presets et un interrupteur. `packages/ui/CLAUDE.md` tient déjà cette frontière pour les 34 boutons non-CTA du dashboard — *« §5 ne décrit pas cette grammaire, et lui inventer des variantes est une décision §8 réservée à Vincent »*. La même phrase vaut ici.
- **Il ne retire pas les alias de compatibilité de `theme.css:58-63`.** Leur condition de retrait écrite est « un sweep cross-repo confirmant zéro référence legacy », et elle n'est pas remplie : `packages/ui` lui-même consomme encore `bg-bg`, `bg-bg-card`, `text-accent`, `text-muted`, `text-heading`. Les retirer aujourd'hui casserait le paquet qui les publie.
- **Il n'ajoute pas `@source` aux consommateurs.** C'est le vol 2 (§ 7).
- **Il ne ferme pas mika#1799.** Le milestone porte le vol 2.

---

## 4. Contrat de vérification

### 4.1 Le détecteur, sur l'arbre réel

| | attendu |
|---|---|
| `bash scripts/check-cta-primitives.sh` | exit **0** |
| sa ligne récapitulative | nomme les **trois** répertoires, et un compte **non nul** mesuré après extension (attendu ≤ 22, § 2 D1) |
| idem, avec D4 annulé (`git stash` du seul `ErrorState.tsx`) | exit **1**, nommant `packages/ui/src/components/ErrorState.tsx:46` et le token `from-accent` |

La troisième ligne est la seule qui prouve quelque chose : *supprimer ce que le test protège, confirmer que le test rougit*. Elle se fait à la main pendant l'implémentation et son résultat va dans le corps de PR.

### 4.2 Le harness — `scripts/test-check-cta-primitives.sh`

Cas ajoutés, chacun sur un arbre jetable à population CTA non nulle (`make_page_with_button`, pour que l'exit 1 ne puisse pas être confondu avec l'exit 3 d'anti-vacuité) :

| cas | fixture | attendu |
|---|---|---|
| **N5** | `<button className="… bg-gradient-to-r from-accent to-accent-light text-white">` | **rouge**, nommant `from-accent` |
| **N5b** | idem en `bg-linear-to-br from-[#7c6af7] to-accent` (orthographe v4) | **rouge** |
| **N6** | `<button role="switch" aria-checked className="… bg-accent">` | **vert** |
| **N6b** | `<button role="button" className="… bg-accent">` | **rouge** — `role="button"` n'est pas un rôle d'état |
| **N7** | `<h1 className="bg-clip-text text-transparent bg-gradient-to-r from-accent to-accent-light">` + un `<button className="bg-clip-text">` | **vert** |
| **N8** | un composant déposé en `packages/ui/src/components/Probe.tsx` portant un `bg-accent` solide | **rouge** — le périmètre atteint vraiment le paquet |
| **N8b** | le même contenu déposé en `packages/ui/src/components/Probe.test.tsx` | **vert** — les fixtures de test sont hors périmètre |
| **N9** | `hover:from-accent`, puis `from-accent/40` | **vert** — les exclusions 1 et 2 dominent le nouveau cas |

N8 demande un `make_ui_component()` jumeau de `make_page()` (même corps, chemin `packages/ui/src/components/`).

L'assertion `assert_says "$REPO_ROOT" "10 CTA element(s)"` (harness l.93) est mise à jour avec le nombre **que le scan filtré rend sur l'arbre**, lu dans sa sortie et non recopié d'ici. La borne d'orientation du § 2 D1 — **au plus 22** — sert à une seule chose : si le nombre lu tombe nettement en dehors (disons au-delà de 25, ou sous 20), le filtre de test ou la détection `<Button>` ne fait pas ce que ce plan croit, et c'est à regarder avant de pinner quoi que ce soit. Pinner un compte qu'on n'a pas compris est la façon dont un harness devient un presse-papier.

### 4.3 Le paquet

| | attendu |
|---|---|
| `npm test --prefix packages/ui` | vert, y compris les assertions `jest-axe` de `ErrorState` |
| `npm run build --prefix packages/ui` | vert |
| `npm run build --prefix dashboard` | vert |
| `theme.test.ts` | vert sans changement de valeur — seuls trois libellés de commentaire bougent |

### 4.4 Le dépôt

`cargo test`, `cargo clippy`, `make lint`, `scripts/verify-pipeline.sh origin/main`.

---

## 5. Ordre de commit — contraint, pas esthétique

Un seul commit, et l'ordre **interne** du diff est contraint par un fait : D1+D2 sans D4 laissent l'arbre rouge, et D4 sans D1+D2 laisse la violation invisible. Les deux doivent atterrir ensemble ou CI casse sur `main`.

1. **D4** — `ErrorState.tsx` migré. L'arbre devient conforme *avant* que quoi que ce soit ne sache le vérifier.
2. **D1 + D2 + D3** — `check-cta-primitives.sh`. Le détecteur étendu trouve zéro violation parce que l'étape 1 l'a précédé.
3. **4.1 ligne 3** — annuler D4 à la main, confirmer le rouge, restaurer. Le résultat va dans le corps de PR.
4. **D2'** — le harness : les huit cas, et le compte mesuré.
5. **D5** — les trois commentaires périmés.
6. **D6** — `packages/ui/CLAUDE.md`.
7. `ErrorState.test.tsx` — assertions (§ 6 du Definition of Done).

L'étape 3 n'est pas un livrable mais elle est dans l'ordre, parce qu'elle est la seule preuve que les étapes 1 et 2 ne se sont pas annulées l'une l'autre.

---

## 6. Risques

| | risque | ce qui le tient |
|---|---|---|
| R1 | **Le prédicat gradient attrape un texte en dégradé.** Le motif `bg-clip-text` + `from-*` est le faux positif naturel de la règle, et sur une landing il est banal. | Mesuré à zéro occurrence sur les trois surfaces avant écriture (§ 2 D2). Fixture N7 vert, qui rougira le jour où la règle dérive vers « tout `from-` ». |
| R2 | **L'exclusion de rôle est écrite trop large** et laisse passer une vraie CTA qui s'est donné un `role`. | La liste est close et ne contient que des rôles à état sélectionné. `role="button"` et `role="link"` en sont exclus, et N6b le tient : une CTA qui déclare `role="button"` reste rouge. |
| R3 | **Le compte d'anti-vacuité devient instable** et le harness rougit sur du travail sain. | C'est précisément ce que l'exclusion des fichiers de test ferme (§ 2 D1). Vérifié neutre sur l'existant : zéro CTA dans les 5 `.test.tsx` du dashboard. |
| R4 | **La migration de `ErrorState` change le rendu** des états d'erreur sur toutes les pages du dashboard. | Changement nommé à l'avance (§ 2 D4 : rayon `lg` → `xl`, padding `sm`), et c'est le rendu que §5 prescrit. La direction est de la non-conformité vers la conformité : un rendu qui change ici est le symptôme de la réparation, pas de la régression. |
| R5 | **La population réelle dépasse la mesure de ce plan** — une violation ou un faux positif de plus, découvert au premier lancement. | Condition d'arrêt écrite dans la Fire-Disposition : halte et remontée opérateur, jamais une entrée d'allowlist, jamais une migration non cadrée. |
| R6 | **Le lien « View error details » perd son `rel`** à la migration et ouvre un `target="_blank"` nu. | `external` produit les deux attributs ensemble (`Button.tsx:135`) ; il n'y a pas d'état où l'un arrive sans l'autre. Assertion dans `ErrorState.test.tsx`. |

---

## 7. Hors périmètre — et le vol 2 qui le porte

**Tailwind ne scanne pas `packages/ui`, et quinze primitives rendent par coïncidence.**

Mesuré par LC.2 le 2026-09-27 et écrit dans `theme.css:134-155` : aucun des deux consommateurs ne déclare d'`@source`, et la détection automatique de contenu de Tailwind v4 est enracinée au root Vite (`dashboard/`, `site/`). Une classe utilitaire qui n'existe que dans ce paquet **n'est jamais générée**. Les primitives qui rendent correctement le font là où la source d'un consommateur contient par hasard la même classe. `w-9`, `top-0.5`, `translate-x-4`, `w-1.5`, `w-1/5`, `outline-offset-*` : mesurées absentes du CSS construit.

Vérifié toujours vrai au 2026-10-19 :

```
$ cat dashboard/src/index.css site/src/index.css | grep -c '@source'
0
```

LC.2 a nommé ce défaut « reported as a follow-up with this measurement ». **Aucun ticket ne le porte** — la recherche sur les tickets ouverts de `senara-solutions/mika` ne rend que #1799 lui-même. C'est la raison de fond pour laquelle #1799 ne se ferme pas avec ce vol : le fermer ferait disparaître le dernier endroit où ce défaut est écrit.

Pourquoi il n'est pas dans ce vol : le réparer signifie ajouter `@source` aux deux consommateurs, ce qui **fait apparaître d'un coup les classes aujourd'hui absentes** — badges, squelettes, interrupteurs changent d'aspect sur l'ensemble du dashboard et de la landing. Le travail n'est pas dans les deux lignes d'`@source` : il est dans la revue visuelle surface par surface qui suit, et dans l'arbitrage de chaque écart (le rendu actuel est-il le bug, ou le rendu complet l'est-il ?). Ce n'est ni le même volume ni la même nature de décision, et une partie relève de §8, donc de Vincent.

**Tracked in: senara-solutions/mika#1799** — le milestone reste ouvert et porte ce vol 2. Le corps de PR de ce vol le nomme.

Deux autres renvois, plus petits :

- **Les alias de compatibilité de `theme.css:58-63`** : condition de retrait non remplie (§ 3). Ils partiront avec le vol 2, qui est le moment où l'on regarde ce que chaque surface consomme réellement.
- **Les 7 boutons non-CTA de `packages/ui`** : frontière §8, non ouverte ici (§ 3).

---

## Definition of Done

1. `scripts/check-cta-primitives.sh` scanne `dashboard/src`, `site/src` et `packages/ui/src`, à l'exclusion de `*.test.tsx`, `*.test.jsx` et `*.type-assertions.tsx`.
2. Un `bg-gradient-to-*` / `bg-linear-*` accompagné d'un stop couleur non translucide et non préfixé de variante est traité comme un fill solide ; le message de violation nomme le stop.
3. Un tag déclarant `role` parmi `switch`, `checkbox`, `radio`, `tab`, `menuitemcheckbox`, `menuitemradio` sort de la population ; `role="button"` et `role="link"` y restent.
4. `ErrorState.tsx` rend ses trois actions via `<Button>` ; plus aucun `bg-gradient`, `text-white`, `target="_blank"` ni `rel=` écrit à la main dans ce fichier.
5. `scripts/test-check-cta-primitives.sh` porte les huit cas N5→N9 du § 4.2 et passe ; son assertion de compte porte le nombre mesuré après extension.
6. `ErrorState.test.tsx` assert : le Retry de liste porte `mika-btn--primary` et `mika-cta-gradient` ; aucun nœud du composant ne porte `text-white` ; le lien détails porte `target="_blank"` **et** `rel="noopener noreferrer"` ; `axe` reste sans violation sur les deux variantes, avec et sans `retry`/`detailsHref`.
7. `scripts/cta-primitives-allowlist.txt` est **inchangé et vide**, et l'assertion du harness qui l'y tient est **inchangée**.
8. Les trois commentaires de `theme.css` / `theme.test.ts` décrivant la contradiction §2/§5.5 constatent sa résolution et citent `76a8b0de` (#1976) ; l'entrée `#ef4444` de `bannedLegacyHexValues` est conservée.
9. `packages/ui/CLAUDE.md` énonce le périmètre à trois répertoires et les deux cas de prédicat ajoutés.
10. Le contrat de vérification du § 4 passe en entier, y compris la ligne 3 du § 4.1 (le détecteur vu rouge sur la violation réelle), dont le résultat figure dans le corps de PR.
11. Le corps de PR porte `Refs #1799`, `Tracked in: senara-solutions/mika#1799` pour le vol 2, et `Code review: delegated to mika-qa (mika#2694)`.
12. mika#1799 **reste ouvert**.

## Acceptance criteria

Le corps de mika#1799 ne porte pas de section `## Acceptance criteria` ; les critères ci-dessous sont dérivés de son § *Priority ladder* item 1 (le seul encore vivant) et du contrat de vérification du § 4.

1. **Il n'existe plus aucune CTA hand-rolled dans `@samidarko/ui`.** `grep -rn --include='*.tsx' 'bg-gradient\|from-accent\|to-accent' packages/ui/src` ne rend rien.
2. **Le bouton « Retry » de `<ErrorState variant="list">` rend la texture §2** — gradient 135° `primary` → `primary_dim`, via `.mika-cta-gradient`, et non un gradient 90° `primary` → `secondary`.
3. **Plus aucun `#ffffff` dans `<ErrorState>`**, conformément à §7 « No Pure White ».
4. **Le détecteur CTA fait feu sur la violation réelle** : avec la migration annulée, `check-cta-primitives.sh` sort en 1 et nomme `packages/ui/src/components/ErrorState.tsx:46`.
5. **Le détecteur ne fait pas feu sur `<LiveRefreshToggle>`** : `check-cta-primitives.sh` sort en 0 sur l'arbre migré, périmètre étendu compris.
6. **L'allowlist est vide et son assertion de vacuité est intacte.** Aucune des deux n'apparaît au diff.
7. **La retenue du détecteur est démontrée par des cas verts** : `role="switch"` + fill, `bg-clip-text` seul, `hover:from-*`, `from-*/40` passent tous.
8. **Le périmètre atteint vraiment le paquet, et pas ses fixtures** : un composant fautif déposé sous `packages/ui/src/components/` fait rougir ; le même contenu en `.test.tsx` ne fait pas rougir.
9. **Aucune valeur de token ne change.** `theme.test.ts` passe sans modification d'une seule paire token/hex.
10. **Le défaut `@source` reste tracké.** mika#1799 est ouvert et le corps de PR le nomme comme porteur du vol 2.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `packages/ui/src/components/ErrorState.tsx` (D4) | 25 |
| `packages/ui/src/components/ErrorState.test.tsx` (DoD 6) | 50 |
| `scripts/check-cta-primitives.sh` (D1+D2+D3, en-tête compris) | 55 |
| `scripts/test-check-cta-primitives.sh` (D2', 8 cas + `make_ui_component`) | 75 |
| `packages/ui/src/theme.css` + `packages/ui/src/theme.test.ts` (D5) | 10 |

Total estimé : 215 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 215 sous 279, avec 64 lignes de marge, et la marge est réelle plutôt qu'optimiste pour trois raisons mesurables :

- **La migration est de taille connue et petite.** La population est de 1, mesurée par grep sur les trois surfaces, et le fichier touché fait 64 lignes au total.
- **Les deux fichiers les plus gros du tableau sont des ajouts par recopie de forme.** Les huit cas du harness suivent le gabarit `assert_exit` / `assert_says` déjà présent 33 fois dans le fichier ; `make_ui_component()` est `make_page()` avec un chemin différent.
- **Aucun des quatre changements du détecteur ne restructure quoi que ce soit.** D1 est une chaîne et un filtre sur un `find` ; D2 et D3 sont deux conditions dans une boucle `awk` qui en contient déjà six ; le reste est de l'en-tête.

Le gros morceau — `@source` et la revue visuelle qui le suit — est précisément ce qui part en vol 2 (§ 7). C'est lui qui aurait fait sauter la borne, et c'est pour ça qu'il n'est pas là.
