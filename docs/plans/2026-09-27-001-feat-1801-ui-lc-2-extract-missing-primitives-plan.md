# LC.2 — extraire les primitives manquantes (mika#1801)

**Ticket :** senara-solutions/mika#1801 — sub-issue de #1799 (milestone Luminescent Core, priorité #2)
**Bloqué par :** LC.1 / mika#1800 — **levé** (`1033424c fix(ui,lc.1): reconcile packages/ui/src/theme.css → rulebook §2`, PR #1972)
**Bloque :** LC.3 (adoption customer-facing)
**Priorité :** p2-normal

---

## Why

Le ticket demande cinq primitives « actuellement dupliquées/divergentes à travers
surfaces ». La lecture du code déplace quatre choses, et ces rectifications sont
**le premier livrable du plan** : elles changent ce qu'il faut faire, et trois
d'entre elles changent ce qu'il ne faut *pas* faire.

### R1 — Le paquet ne s'appelle pas `@senara-solutions/ui`

Le ticket l'écrit ainsi dans son scope et dans son AC1. Le paquet réel est
**`@samidarko/ui`** (`packages/ui/package.json:2`), renommé et publié en accès
public par mika#1386. C'est la même rectification que LC.5 a dû faire
(`docs/plans/2026-09-20-003-…-1804-…-plan.md` § R1) ; elle est citée ici comme
précédent, pas redécouverte. Un plan qui reprend le nom du ticket produit un
`npm install` qui échoue.

### R2 — Le split de `Button` annoncé n'existe pas ; la divergence réelle est ailleurs, et elle est plus grave

Le ticket décrit « des variantes inconsistentes : `bg-accent/90` vs
`bg-accent-light` vs gradient ». Mesuré sur l'arbre à `7c373d01` :

| forme annoncée | occurrences réelles | lecture |
|---|---|---|
| `bg-accent/90` | **0** | n'existe nulle part dans le dépôt |
| gradient sur un bouton | **0** | les 5 `gradient` du dépôt sont des fonds de section et un dégradé SVG de graphe |
| `bg-accent-light` | 2, et ce ne sont **pas** des variantes | c'est l'état `hover:` des deux boutons primaires du dashboard |

Il n'y a donc pas trois variantes en concurrence. Il y en a **une seule**, un
`bg-accent` plat, implémentée de façon cohérente sur les deux surfaces. La
divergence réelle est d'un autre ordre : **cette forme unique contredit le
rulebook**, qui prescrit noir sur blanc (`docs/design/luminescent-core.md:105`) :

> **Primary:** Gradient fill (`primary` to `primary_dim`), `xl` (1.5rem)
> roundedness. No border.

et, en §2 « Signature Textures » (l.44) :

> Main CTAs must use a linear gradient: `primary` (#ada3ff) to `primary_dim`
> (#715eeb) at a 135-degree angle. This provides a "soul" to the interface that
> a flat hex code cannot replicate.

**Aucun CTA du dépôt n'est un gradient.** Ce n'est pas une dérive entre surfaces
à réconcilier l'une sur l'autre — c'est une règle écrite que personne n'applique.
Conséquence directe sur la conduite : extraire `<Button>` en figeant la forme
actuelle produirait une primitive *canonique* qui viole §5 de façon désormais
centralisée et durable. L'extraction et l'alignement sur le rulebook sont le même
geste, et les séparer serait poser le mauvais contrat dans le paquet partagé.

Population CTA complète, mesurée et exhaustive (prédicat : élément interactif
`<button>` ou `<a href>` portant un fond plein) :

| # | site | forme actuelle | variante §5 |
|---|---|---|---|
| 1 | `dashboard/src/pages/Timeline.tsx:94` | `bg-accent text-white … hover:bg-accent-light` | Primary |
| 2 | `dashboard/src/pages/Traces.tsx:41` | idem + `disabled:opacity-30 disabled:cursor-not-allowed` | Primary |
| 3 | `dashboard/src/pages/SkillVariants.tsx:308` | `bg-green-600 text-white hover:bg-green-500 disabled:opacity-50` | Primary (**token violé** — voir D4) |
| 4 | `site/src/components/Hero.tsx:45` | `bg-accent … shadow-lg shadow-accent/20 hover:-translate-y-0.5` | Primary, sur un `<a>` |
| 5 | `site/src/components/Nav.tsx:36` | `rounded-full bg-accent … hover:shadow-[…]` | Primary, sur un `<a>` |
| 6 | `site/src/components/Hero.tsx:51` | `border border-white/10 bg-white/[0.03]` | Secondary, sur un `<a>` |
| 7 | `dashboard/src/pages/SkillVariants.tsx:130` | `border border-white/[0.1] text-muted hover:…` | Secondary |
| 8 | `dashboard/src/pages/SkillVariants.tsx:~300` | `px-4 py-2 text-sm text-muted hover:text-heading` | Tertiary (annuler) |

Les 34 autres `<button>` du dashboard ne sont **pas** des CTA : ce sont des
boutons-icône (`p-1 rounded hover:bg-…`), des bascules de section repliable et
des liens-texte. Ils relèvent d'une grammaire que le rulebook ne décrit pas, et
les faire entrer dans `<Button>` serait élargir le périmètre sur une population
que §5 ne couvre pas. Ils sont hors périmètre, nommément (voir *Out of scope*).

### R3 — Trois des cinq primitives ont une population de **zéro** dans ce dépôt

C'est la rectification décisive, et elle ne se devine pas depuis le corps du
ticket. Mesuré :

| primitive | occurrences dans `dashboard/` + `site/` + `docs-site/` | règle rulebook |
|---|---|---|
| **`Button`** | **8 callsites, 2 surfaces** (tableau ci-dessus) | §5 « Buttons » — existe, précise, non appliquée |
| **`Spinner`** | 3 usages **dans un unique fichier** (`InvestigationPanel.tsx:357,379,420`), tous identiques : `<Loader2 className="animate-spin" />` | **aucune** (voir R4) |
| **`PasswordInput` / `SecretField`** | **0** — `grep 'type="password"'` ne rend rien, dans aucune extension, sur tout le dépôt | §5 « Input Fields » existe mais ne dit rien du masking/reveal/copy |
| **`AuthCard` / `CenteredCard`** | **0** — aucun flow login/signup/invite ; les seules occurrences de « auth » sont des en-têtes HTTP `Authorization` dans `dashboard/src/api/` | §5 « Cards & Lists » existe, rien sur les layouts auth |
| **`Stepper`** | **0** — `grep -i 'stepper\|currentStep'` ne rend rien | **aucune** |

Les trois primitives à population nulle sont exactement celles des flows
**customer-facing** : login, signup, invite, onboarding. Et le rulebook dit où
ils vivent (`luminescent-core.md:322`, §9 Implementation surface) :

> **`mika-cloud/`** — Cloud Console (frontend lives in `mika-cloud`, gateway code
> in `mika/crates/mika-gateway`). Consumes `@samidarko/ui`.

`mika-cloud` est un dépôt fermé, **absent de ce workspace**. Le ticket le dit
lui-même sans en tirer la conséquence : son § Suivi annonce que LC.2 « bloque LC.3
(customer-facing adoption) ». Ces trois primitives n'ont pas d'appelant ici et
n'en auront pas : leur consommateur est ailleurs, et on ne peut ni le lire, ni le
compiler, ni le tester depuis ce dépôt.

Deux conséquences, et aucune n'est cosmétique :

1. **Concevoir l'API sans appelant, c'est la deviner.** Un `<Stepper>` a-t-il un
   état `error` par étape ? Les étapes sont-elles cliquables en arrière ? Le
   ticket ne le dit pas, aucun code ne le montre, et une API de primitive
   partagée figée à l'aveugle se paie à chaque consommateur qui la contourne.
2. **Chaque primitive de ce paquet devient une règle d'enforcement.**
   `packages/ui/CLAUDE.md` § Enforcement Rules transforme systématiquement une
   primitive en « hand-rolled X is a review fail ». Trois primitives sans
   population produiraient trois règles sans population — un détecteur
   silencieusement inerte se lit exactement comme un arbre propre (classe
   mika#2205, nommée à répétition dans `CLAUDE.md`).

Et la procédure du rulebook l'interdit pour `Stepper` (`luminescent-core.md:305`,
§8 Extension policy) :

> 1. A surface needs a pattern not in this document.
> 2. The pattern is **proposed to Vincent** […]
> 3. If accepted, the pattern is added to this document **with a section
>    identifying which surfaces consume it**.

On ne peut satisfaire ni le point 1 (aucune surface d'ici n'en a besoin) ni le
point 3 (on ne peut pas nommer les surfaces consommatrices). Et §9 pose le
critère de partage : « **if more than one surface needs it**, it goes in
`@samidarko/ui` » — un critère qu'on ne peut pas établir à une surface près,
puisqu'on ne peut pas en établir **une**.

La conduite qui en découle est en D5, avec son coût nommé et les deux gestes qui
la débloquent.

### R4 — « Spinner (rulebook §6 sizes) » est une référence fausse

§6 est « Roundness & Spacing » : rayons d'angle et échelle de 8px. Elle ne
mentionne aucun spinner, aucune taille de loader. `grep -n Spinner` sur les trois
documents de `docs/design/` rend **zéro ligne**. Il n'y a pas d'échelle de tailles
de spinner à implémenter : elle est à poser, ce qui relève de §8 comme le reste.

Ce qui sauve `Spinner` n'est donc pas sa population propre (une surface, un
fichier — sous le seuil de §9), c'est qu'il est une **dépendance de `Button`** :
l'AC1 du ticket demande `loading` « avec Spinner intégré ». Un spinner extrait
comme sous-composant de `Button`, avec un jeu de tailles calé sur les tailles de
bouton plutôt que sur une échelle §6 inexistante, est justifié par l'appelant qui
l'exige. Un spinner extrait « parce que la liste le dit » ne l'est pas.

### R5 — Aucun bouton du dépôt n'a d'état `loading`

`loading` est demandé par le ticket ; sa population mesurée est **0**. Ce n'est
pas une raison de ne pas le livrer — `Traces.tsx` et `SkillVariants.tsx` portent
tous deux un `disabled` piloté par une mutation (`promoteMutation.isPending`) et
sont les appelants naturels du premier jour. C'est une raison de le dire : c'est
la seule prop de `<Button>` dont le comportement n'est adossé à aucun usage
existant, donc la seule dont la migration ne la vérifiera pas.

### R6 — `site/` n'est ni compilé ni testé par la CI

`ci.yml` build `packages/ui` + `dashboard` (job *Dashboard*, l.137-141) et
`docs-site` (job *Docs Site*, l.157). `site/` n'a **que** `check-landing-tokens.sh`,
un grep. Le commentaire du job le dit en toutes lettres (`ci.yml:321`) :

> `site/` has no other automated check at all.

Or `site/` est dans les workspaces npm racine (`package.json:5`), porte un
`"build": "tsc -b && vite build"` et déclare `@samidarko/ui: "*"` — dont il
n'importe aujourd'hui **que le CSS** (`site/src/index.css:2`), jamais un
composant. Migrer la landing vers `<Button>` y ferait donc entrer le premier
import de composant TypeScript, dans la seule surface où une erreur de type ou
d'import **passerait le merge sans rougir**. Faire entrer `site` dans le job de
build est une précondition de la migration de la landing, pas un supplément.

---

## Requirements

1. **R-1** — `<Button>` est extrait dans `@samidarko/ui` et implémente les trois
   variantes du rulebook §5 : `primary` (gradient `primary`→`primary_dim` à 135°),
   `secondary` (ghost, bordure `outline_variant` à 20 %, hover vers
   `secondary_container`), `tertiary` (texte `primary`, sans fond).
2. **R-2** — `<Button>` supporte les deux éléments rendus mesurés : `<button>`
   (dashboard) et `<a href>` (landing). Les props qui n'ont pas de sens sur un
   lien (`disabled`, `loading`, `type`) sont **inatteignables** sur cette branche
   de l'API, pas seulement ignorées.
3. **R-3** — `<Spinner>` est extrait, consommé par `<Button loading>`, et
   exportable seul.
4. **R-4** — Les 8 callsites CTA mesurés en R2 sont migrés. Aucun ne conserve de
   fond hand-rolled.
5. **R-5** — Un détecteur refuse un nouveau CTA hand-rolled dans `dashboard/src`
   et `site/src`.
6. **R-6** — `site/` entre dans un job de build CI (précondition de R-4 sur la
   landing, cf. R6 du *Why*).
7. **R-7** — Chaque primitive livrée porte ≥ 1 test de comportement **et** une
   assertion `jest-axe`, conformément au standard de `packages/ui/CLAUDE.md`
   § Accessibility Standards.
8. **R-8** — `PasswordInput`, `AuthCard`, `Stepper` : la conduite de D5 est
   appliquée et sa remontée est écrite dans le corps de la PR.
9. **R-9** — `packages/ui/CLAUDE.md` (tableau des primitives + règles
   d'enforcement) et `docs/design/luminescent-core.md` §9 sont mis à jour pour ce
   qui est livré, et **seulement** pour ce qui est livré.
10. **R-10** — Bump mineur de `@samidarko/ui` (0.3.1 → 0.4.0).

---

## Design

### D1 — L'API de `<Button>` : union discriminée sur l'**élément**, pas sur la variante

L'AC3 du ticket demande « discriminated unions où sensé ». Le réflexe serait de
discriminer sur `variant`, à l'image de `<ListRow>`. **C'est le mauvais axe ici**,
et la mesure le montre : les trois variantes du rulebook partagent exactement les
mêmes props (un label, une icône éventuelle, un état). Une union sur `variant`
serait trois fois la même interface sous trois noms — de la cérémonie, pas un
contrat.

L'axe où la discrimination porte de l'information est l'**élément rendu**, et R2
l'établit par la mesure : quatre CTA sur huit sont des `<a href>` (landing), les
autres des `<button>`. Or `disabled` n'existe pas sur un `<a>` en HTML, et un lien
« en chargement » n'a pas de sens. Un sac de props optionnelles laisserait écrire
`<Button href="/x" loading disabled />` — trois mots qui se contredisent, que le
compilateur accepterait, et qui rendraient un `<a>` inerte non annoncé aux
technologies d'assistance. L'union le rend inexprimable :

```ts
type ButtonVariant = 'primary' | 'secondary' | 'tertiary'
type ButtonSize = 'sm' | 'md' | 'lg'

interface ButtonBaseProps {
  variant?: ButtonVariant        // défaut 'primary'
  size?: ButtonSize              // défaut 'md'
  icon?: ReactNode               // décoratif → aria-hidden posé par la primitive
  children: ReactNode
  className?: string
}

interface ButtonActionProps extends ButtonBaseProps {
  as?: 'button'
  onClick: () => void
  type?: 'button' | 'submit'
  disabled?: boolean
  loading?: boolean              // implique disabled ; rend <Spinner> à la place de icon
}

interface ButtonLinkProps extends ButtonBaseProps {
  as: 'link'
  href: string
  external?: boolean             // pose target="_blank" rel="noopener noreferrer"
}

type ButtonProps = ButtonActionProps | ButtonLinkProps
```

`as` est optionnel côté action pour que le cas majoritaire reste `<Button
onClick={…}>`. Le discriminant est donc *présent* uniquement sur la branche
minoritaire — même économie que `<ListRow>`, qui impose `variant` parce qu'aucune
de ses trois branches n'est majoritaire.

**Choix explicite : `loading` implique `disabled`.** Les deux appelants mesurés
(`Traces.tsx`, `SkillVariants.tsx`) pilotent déjà `disabled` sur un état de
mutation ; laisser les deux props indépendantes autoriserait un bouton en
chargement encore cliquable, c'est-à-dire la double-soumission que l'état existe
pour empêcher. Le rendu pose `aria-busy="true"` et conserve la largeur du bouton
(le spinner remplace l'icône, pas le label) pour ne pas faire sauter la mise en
page — la régression classique d'un bouton qui rétrécit pendant sa requête.

### D2 — Le gradient, et où il est écrit

§5 prescrit `primary` → `primary_dim` à 135°. Les deux tokens existent déjà dans
`theme.css` (`--color-primary: #ada3ff`, `--color-primary-dim: #715eeb`, posés par
LC.1). Tailwind v4 n'a pas d'utilitaire pour un dégradé à 135° entre deux
variables CSS arbitraires sans arbitrary-value verbeuse répétée à chaque site ; la
forme juste est une classe utilitaire déclarée **une fois** dans `theme.css` à
côté des tokens qu'elle consomme :

```css
/* Rulebook §2 "Signature Textures" + §5 Buttons — la seule texture de CTA. */
.mika-cta-gradient {
  background-image: linear-gradient(135deg, var(--color-primary), var(--color-primary-dim));
}
```

Elle vit dans `theme.css` parce que c'est le fichier que les trois surfaces
importent déjà, y compris `site/` qui n'importe rien d'autre — donc la landing
obtient la texture même sur les CTA que l'implémentation n'atteindrait pas. Et
parce que `theme.test.ts` y assertionne déjà par lecture de fichier : la texture
devient pinnable par le même mécanisme, au même endroit que les valeurs qu'elle
compose. Écrire le dégradé dans le TSX de `<Button>` le rendrait invisible à cette
garde et le dupliquerait le jour où une seconde surface en a besoin.

**Effet visible, à dire plutôt qu'à découvrir en revue :** les 5 CTA primaires
changent d'apparence. Ils passent d'un aplat `#ada3ff` à un dégradé
`#ada3ff → #715eeb`. C'est l'objet du ticket (« unifier »), c'est ce que le
rulebook demande depuis son écriture, et c'est la première fois qu'une surface
l'applique. Le hover perd `bg-accent-light` au profit d'un assombrissement du
dégradé — §5 ne prescrit pas de hover pour le primaire, donc c'est une décision
d'implémentation à nommer dans la PR, pas une règle à inventer dans le rulebook.

### D3 — `<Spinner>`

```ts
interface SpinnerProps {
  size?: 'xs' | 'sm' | 'md'   // 12 / 16 / 20 px — calés sur les tailles de Button
  className?: string
  ariaLabel?: string          // défaut 'Loading'
}
```

Implémenté sur `<Loader2>` de `lucide-react` + `animate-spin`, c'est-à-dire
exactement la forme déjà présente dans `InvestigationPanel.tsx` — l'extraction ne
change aucun pixel sur ce fichier, elle lui retire une répétition.

`role="status"` + `aria-label` : un spinner sans nom accessible est invisible aux
lecteurs d'écran, et `packages/ui/CLAUDE.md` l'exige déjà pour les changements
d'état asynchrones. Quand il est rendu **à l'intérieur** de `<Button loading>`, le
`role="status"` est retiré et c'est le bouton qui porte `aria-busy` : deux régions
live imbriquées annonceraient deux fois le même fait.

Les trois tailles sont calées sur `ButtonSize` et non sur une échelle §6 (qui
n'existe pas, cf. R4). Les valeurs mesurées dans `InvestigationPanel` sont 9, 12
et 16 px ; l'échelle proposée les couvre à un cran près, et l'implémentation
conserve `size={9}` en `className` local si l'écart est visible — un badge de 9 px
n'est pas une taille de primitive.

### D4 — Le bouton vert de `SkillVariants.tsx:308`

C'est le seul CTA du dépôt dont la couleur n'est pas `accent`. Sa classe est
`bg-green-600 … hover:bg-green-500` — deux valeurs qui **n'existent dans aucun
token** : ni `theme.css`, ni le rulebook. C'est déjà une violation de la règle
« Design tokens over hardcoded colors » de `packages/ui/CLAUDE.md`, indépendante
de ce ticket.

Il est migré en `variant="primary"`, et **il perd son vert**. Le raisonnement est
celui du rulebook lui-même (§8, l.314) :

> The rulebook never splits. We never have "the Dashboard's version" of a button.
> If a button needs a variant, the variant is added to this document and offered
> to all surfaces.

Ajouter une quatrième variante `confirm` serait une décision produit que §8 réserve
à Vincent, et la prendre à l'implémentation ferait entrer dans le paquet partagé
une variante qu'un seul bouton consomme. La perte de signal est nommée dans le
corps de la PR ; si Vincent veut une variante `confirm`/`destructive`, elle
s'ajoute au rulebook puis à la primitive, dans cet ordre et dans son ticket.

Cet arbitrage a une conséquence directe sur la Fire-Disposition : il rend la
population du détecteur **vide** après migration, donc l'allowlist livrable vide.

### D5 — `PasswordInput`, `AuthCard`, `Stepper` : halte-et-remontée

**Ces trois primitives ne sont pas livrées, et ce n'est pas un rétrécissement de
périmètre décidé en chemin : c'est un blocage établi par mesure, remonté avec son
geste de levée.** R3 en donne les trois raisons — population nulle dans ce dépôt,
consommateur dans `mika-cloud` (hors workspace), et interdiction procédurale de
§8 pour un pattern que le rulebook ne décrit pas.

Ce que l'implémentation livre à leur place :

1. **Une section nommée dans le corps de la PR** — « Trois primitives sur cinq ne
   sont pas livrées » — portant le tableau de mesure de R3, la citation de §8/§9,
   et les deux gestes qui débloquent.
2. **Un ticket de suivi**, `LC.2b — primitives customer-facing (PasswordInput,
   AuthCard, Stepper)`, avec sa précondition écrite : *au moins un callsite réel
   lisible, ou une section de rulebook décrivant le pattern.*
3. **Aucune règle d'enforcement** ajoutée à `packages/ui/CLAUDE.md` pour ces
   trois-là. C'est le point qui compte : une règle « hand-rolled Stepper is a
   review fail » sans un seul Stepper dans l'arbre est une garde sans population,
   et son silence se lirait comme une conformité.

**Les deux gestes qui lèvent le blocage**, l'un ou l'autre suffisant :

- **(g1)** Vincent ajoute au rulebook les sections décrivant les trois patterns,
  en nommant les surfaces consommatrices (§8.3). L'API cesse alors d'être devinée :
  elle est dérivée d'une règle écrite, exactement comme `<Button>` l'est ici de §5.
- **(g2)** LC.3 / `mika-cloud` fournit les callsites — même par simple lecture d'un
  fichier de la console. Un appelant réel suffit à fixer l'API ; il n'a pas besoin
  d'être dans ce dépôt pour être lu.

**Coût assumé, écrit plutôt que découvert :** l'AC1 du ticket (« 5 primitives
extraites ») **n'est pas satisfaite à la lettre** par ce plan. Deux sur cinq le
sont. L'alternative — livrer les cinq — a un coût qui n'est pas moindre, il est
seulement moins visible : trois API conçues sans appelant, trois règles
d'enforcement sans population, et un `Stepper` dans le paquet partagé en
contournement de la procédure que le rulebook réserve à son propriétaire. Si
l'opérateur préfère cette voie, elle est exécutable : le plan la nomme ici plutôt
que de la taire, et la décision lui revient.

### D6 — `site/` entre dans la CI

Une ligne dans le job *Dashboard* de `ci.yml`, après le build du dashboard :

```yaml
      - run: npm run build -w site
```

`site` est déjà un workspace npm racine et porte `tsc -b && vite build` : le
`npm ci` du job l'installe déjà. Le coût est un `tsc` sur douze fichiers.

Sans cette ligne, R-4 sur la landing est livré **non vérifié** : c'est la seule
surface du dépôt où un import cassé passe le merge, et ce PR y introduit son
premier import de composant.

### D7 — Ordre d'implémentation

L'ordre n'est pas indifférent : chaque étape rend la suivante vérifiable.

1. `.mika-cta-gradient` dans `theme.css` + assertion dans `theme.test.ts`.
2. `<Spinner>` + tests (comportement + axe).
3. `<Button>` + tests, consommant `<Spinner>`.
4. Exports dans `packages/ui/src/index.ts`, bump 0.4.0.
5. `npm run build -w site` dans `ci.yml` — **avant** la migration de la landing.
6. Migration des 8 callsites (dashboard puis landing).
7. Détecteur `scripts/check-cta-primitives.sh` + son contrôle négatif + job CI.
8. Docs : `packages/ui/CLAUDE.md`, `luminescent-core.md` §9.

Le détecteur arrive **après** la migration (7 après 6) parce que c'est ce qui lui
permet d'atterrir armé avec une allowlist vide. Armé avant, il rougirait sur les
huit callsites que le même PR est en train de migrer.

---

## Fire-Disposition

Ce plan livre un détecteur : `scripts/check-cta-primitives.sh` (R-5), dont le
chemin de succès est « aucun CTA hand-rolled trouvé ». La section est donc
requise.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est livrée
VIDE.**

Elle est vide parce que D4 tranche le seul cas qui aurait exigé une entrée : le
bouton vert de `SkillVariants.tsx:308` est migré en `primary` plutôt
qu'allowlisté, puisque ses couleurs ne sont de toute façon dans aucun token. Les
sept autres callsites sont migrés par R-4. La population post-migration est donc
nulle, et l'allowlist n'a rien à nommer.

Le fichier `scripts/cta-primitives-allowlist.txt` est **créé vide** (avec son
en-tête documentaire) plutôt qu'omis, pour trois raisons :

1. **La doctrine de résolution est écrite à l'avance.** L'en-tête dit, comme
   `scripts/pilot-push-allowlist.txt` le fait pour mika#2520 : *quand le scan
   tire, on route le site vers `<Button>` ; on n'ajoute pas de ligne ici.* Une
   allowlist dont la règle d'usage n'est écrite qu'au moment où on en a besoin est
   une allowlist qui se remplit.
2. **La comparaison est double sens.** Une entrée qui ne matche plus rien fait
   rougir le build — l'assertion auto-nettoyante exigée par mika#1574 (a). Sans
   elle, une exception survivrait à la disparition de sa cause et exempterait en
   silence un futur homonyme.
3. **Un test pinne l'allowlist vide.** `test-check-cta-primitives.sh` assertionne
   `wc -l == 0` sur les lignes non-commentaires. Le jour où quelqu'un y ajoute une
   ligne, il doit d'abord retirer cette assertion — c'est-à-dire prendre la
   décision explicitement, et non l'accumuler.

**Anti-vacuité — le scan doit prouver qu'il regarde quelque chose.** C'est la
moitié que la maison a dû réapprendre plusieurs fois (mika#2205, mika#2496 terme
5, mika#2420 halte 4) : un scan devenu inerte par renommage de répertoire rend
exit 0 exactement comme un arbre propre. Le scan imprime donc
`N files scanned, M CTA elements found, 0 violations` et **échoue si N ou M vaut
zéro** — huit CTA sont connus, migrés ils restent huit CTA, simplement rendus par
`<Button>`. Le contrôle positif est donc comptable : le scan doit trouver les
huit `<Button>`.

**Le prédicat porte sur l'élément JSX complet, jamais sur la ligne.** Mesuré :
`Traces.tsx` ouvre son `<button` en l.38 et écrit son `className` en l.41. Un
prédicat ancré à la ligne raterait la moitié de la population — c'est le piège
exact que mika#2496 a dû nommer en terme 1 pour `dispatch-lib.sh`. Le scan joint
donc chaque élément interactif à son attribut de classe avant d'évaluer.

Quatre fixtures de contrôle négatif, chacune **vue rouge ou verte** avant merge :

| fixture | attendu | ce qu'elle atteste |
|---|---|---|
| N1 — `<button className="… bg-accent text-white …">` sur une ligne | **rouge** | le cas de base mord |
| N2 — même bouton, `className` sur une ligne suivante | **rouge** | l'unité d'analyse est l'élément, pas la ligne |
| N3 — `<div className="… bg-accent …">` (le dot de `Nav.tsx:13`, le numéro d'étape de `HowItWorks.tsx:66`) | **vert** | le non-interactif est hors population — sans quoi le scan serait rouge en permanence, donc désarmé |
| N4 — `<span className="bg-accent/10 …">` (chip de `Tasks.tsx:250`) | **vert** | `bg-accent/N` est un fond de chip, pas un CTA |

N3 et N4 ne sont pas décoratives : sans elles, « le scan attrape les CTA » est
indistinguable de « le scan rougit sur toute occurrence de `bg-accent` », et la
seconde forme serait mise en sourdine dans la semaine.

---

## Verification contract

| # | vérification | commande | attendu |
|---|---|---|---|
| V1 | Les primitives passent leurs tests + axe | `npm test -w packages/ui` | vert, ≥ 2 nouveaux fichiers de test |
| V2 | La texture de CTA est pinnée | `npm test -w packages/ui` (`theme.test.ts`) | l'assertion `.mika-cta-gradient` passe |
| V3 | Le dashboard compile et passe | `npm run lint -w dashboard && npm run build -w dashboard && npm test -w dashboard` | vert |
| V4 | **La landing compile** (nouveau, D6) | `npm run build -w site` | vert |
| V5 | Les tokens de la landing sont intacts | `bash scripts/check-landing-tokens.sh` | exit 0 — la migration n'introduit aucun littéral |
| V6 | Le détecteur est propre **et non vacuous** | `bash scripts/check-cta-primitives.sh` | exit 0, et la ligne de compte porte `8 CTA elements found` |
| V7 | Le détecteur sait échouer | `bash scripts/test-check-cta-primitives.sh` | les 4 fixtures rendent le verdict du tableau ci-dessus |
| V8 | L'allowlist est vide | inclus dans V7 | 0 ligne non-commentaire |
| V9 | Zéro CTA hand-rolled résiduel | `grep -rnE '<(button\|a)\b' dashboard/src site/src` recoupé avec V6 | les 8 callsites de R2 rendent `<Button>` |

**Vérification visuelle, et elle n'est pas automatisable :** D2 change
l'apparence des 5 CTA primaires. `npm run dev:dashboard` et `npm run dev -w site`,
capture avant/après dans le corps de la PR. Un test ne peut pas dire qu'un dégradé
à 135° est le bon dégradé.

---

## Definition of Done

- [ ] `<Button>` et `<Spinner>` livrés dans `packages/ui/src/components/`, exportés
      depuis `index.ts`.
- [ ] `<Button>` implémente les trois variantes de §5, avec le gradient de §2 sur
      `primary`.
- [ ] L'union discriminée rend `<Button as="link" disabled>` **non compilable**.
- [ ] Les 8 callsites de R2 migrés ; aucun fond de CTA hand-rolled restant.
- [ ] `.mika-cta-gradient` dans `theme.css`, pinné par `theme.test.ts`.
- [ ] `scripts/check-cta-primitives.sh` + `test-check-cta-primitives.sh` + job CI,
      allowlist vide et pinnée vide.
- [ ] `npm run build -w site` dans le job *Dashboard* de `ci.yml`.
- [ ] `@samidarko/ui` bumpé en 0.4.0.
- [ ] `packages/ui/CLAUDE.md` : `<Button>` et `<Spinner>` dans le tableau,
      règle d'enforcement pour les CTA. **Rien pour les trois non livrées.**
- [ ] `luminescent-core.md` §9 : la ligne d'implémentation de §5 Buttons pointe
      la primitive.
- [ ] Corps de PR : section « Trois primitives sur cinq ne sont pas livrées »
      (D5), capture avant/après des CTA, et la perte du vert de D4.
- [ ] Ticket de suivi LC.2b ouvert avec sa précondition.
- [ ] V1–V9 verts.

---

## Acceptance criteria

Transcrits du corps de mika#1801, avec leur statut sous ce plan.

1. **« `Button` et `Spinner` extraits dans `@samidarko/ui`, CTA migrés, détecteur
   armé »** — satisfait. Cet AC1 a été **réécrit et ratifié le 2026-09-28**
   (opérateur-proxy, découpage de périmètre) : sa version d'origine demandait les
   5 primitives. Les trois autres (`PasswordInput`, `AuthCard`, `Stepper`) ont une
   population mesurée de zéro dans ce dépôt (R3), leur consommateur vit dans
   `mika-cloud` ; elles relèvent de mika#2562. Le nom du paquet est
   `@samidarko/ui` (R1).
2. **« Chaque primitive : tests behavior + jest-axe passing »** — satisfait pour
   les primitives livrées (R-7, V1).
3. **« TypeScript types stricts, discriminated unions où sensé »** — satisfait :
   l'union discrimine sur l'élément rendu, qui est l'axe où elle porte de
   l'information, et non sur `variant`, où elle n'en porterait aucune (D1).
4. **« Doc examples README ou stories »** — satisfait par le tableau et les
   patrons d'appel de `packages/ui/CLAUDE.md`, qui est la surface de documentation
   établie du paquet (aucun Storybook dans la stack ; aucun README de composant
   n'existe aujourd'hui). Créer une seconde surface de doc pour deux composants
   diviserait celle qui existe.
5. **« Bump version `@senara-solutions/ui` (semver minor) »** — satisfait :
   `@samidarko/ui` 0.3.1 → 0.4.0.

---

## Out of scope

- **Les 34 boutons non-CTA du dashboard** (boutons-icône, bascules de section,
  liens-texte). Le rulebook §5 ne décrit pas cette grammaire ; les faire entrer
  dans `<Button>` serait inventer des variantes que §8 réserve à Vincent. Ils
  restent locaux, et le détecteur les laisse passer par construction (fixture N3).
- **Une variante `confirm`/`destructive`** (D4). Décision produit, §8, ticket
  séparé si Vincent la veut.
- **Les littéraux `text-red-400` / `text-green-400`** du dashboard (12+
  occurrences mesurées dans `VariantDiffViewer`, `DevRunDetail`, `LlmCallDetail`,
  `InvestigationPanel`). Même famille de défaut que celui que LC.5 a fermé sur la
  landing, mais population, surface et remède différents — c'est le périmètre d'un
  `check-dashboard-tokens.sh`, pas de celui-ci.
- **`docs-site/`** — Astro + Starlight, zéro `<button>`, absent de §9. Ce n'est pas
  une surface du design system.
- **`mika-cloud`** — hors de ce workspace, structurellement inatteignable.
- **La contradiction interne du rulebook sur `error`** (§2 `#ff6e84` vs §5.5
  `#ef4444`), déjà remontée à Vincent par LC.1 et notée dans `theme.css:41-44`.
- **L'adoption customer-facing** — c'est LC.3, ce que ce ticket débloque.

---

## Risques

| # | risque | probabilité | atténuation |
|---|---|---|---|
| 1 | **Le gradient déplaît visuellement.** C'est la première application de §2 « Signature Textures » ; personne n'a vu le résultat sur ces surfaces. | moyenne | Capture avant/après dans la PR, et la texture est **une déclaration dans `theme.css`** : la retirer ou la régler est une ligne, pas une rétro-migration de huit callsites. |
| 2 | **La landing casse sans que la CI le voie.** Premier import de composant dans une surface non compilée (R6). | haute si D6 est omis | D6 est ordonnancé **avant** la migration de la landing (D7 étape 5 avant 6). Si D6 échoue, la landing n'est pas migrée dans ce PR. |
| 3 | **Le détecteur produit des faux positifs** sur les `bg-accent` non-interactifs (dots, numéros d'étape, icônes de feature — 6 occurrences mesurées). | moyenne | Fixture N3 vue verte avant merge. Un faux positif ici coûte un PR bloqué et se solde par une mise en sourdine ; c'est le mode de panne à fermer en premier. |
| 4 | **L'architecte ou l'opérateur refuse D5** et veut les cinq primitives. | réelle | Le plan nomme la voie et son coût plutôt que de la taire ; le périmètre s'élargit alors sans rien invalider de ce qui est écrit ici, `Button` et `Spinner` restant inchangés. |
| 5 | **`loading` livré non vérifié par un appelant réel** (R5). | certaine | Migrer `SkillVariants.tsx:308` en `loading={promoteMutation.isPending}` plutôt qu'en `disabled` — un appelant réel existe, il suffit de le brancher. C'est la seule atténuation disponible et elle est suffisante. |
| 6 | **Un consommateur de `mika-cloud` pinne 0.3.x** et rate le bump. | faible | Bump mineur, aucune rupture : rien n'est retiré du paquet. |

---

## Revision history

| date | révision |
|---|---|
| 2026-09-27 | Rédaction initiale. Quatre rectifications mesurées portées en tête (R1–R6) : le nom du paquet, l'inexistence du split `Button` annoncé, la population nulle de trois primitives sur cinq, la fausse référence §6 pour `Spinner`, et l'absence de toute compilation CI de `site/`. |
