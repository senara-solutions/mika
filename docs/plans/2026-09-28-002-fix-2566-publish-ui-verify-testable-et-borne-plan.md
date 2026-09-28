# mika#2566 — la vérification post-publish devient testable, bornée et auto-diagnostique

**Ticket :** `senara-solutions/mika#2566` — `ci(publish-ui): la vérif post-publish (5×15 s) échoue alors que la version est publiée — faux rouge sur main`
**Type :** fix (CI)
**Labels :** `bug`, `p3-nice-to-have`, `ready`, `dispatch:loop`

---

## Le défaut, mesuré

Run 36410711981 sur `main`, 2026-09-28, après le merge de mika#2556 (bump
`@samidarko/ui` 0.3.1 → 0.4.0). L'étape « Verify publish (with CDN-propagation
retry) » a rendu `Attempt 1/5` … `Attempt 5/5` puis
`Post-publish verification failed after 5 attempts`. Quelques minutes plus tard,
`npm view @samidarko/ui version` rendait `0.4.0` : **la publication avait
réussi**.

Conséquence, et le ticket la nomme mieux que ne le ferait un résumé : on lit le
rouge comme une publication ratée, ou on apprend à l'ignorer, et *ce second effet
est pire* — le jour où la publication échoue vraiment, le rouge ne dit plus rien.

---

## Ce que la lecture du code déplace, et c'est le premier livrable

**Le remède proposé par le ticket — allonger la fenêtre — EST le remède qui a
déjà échoué.** `git log .github/workflows/publish-ui.yml` rend `d32a0fd7`,
« chore(ci): publish-ui verify step retry+backoff for CDN propagation lag
(dette #1917) », et le commentaire de l'étape cite son propre incident fondateur :

```
# npmjs CDN can take up to ~60s to propagate a new publish. Retry with
# backoff to avoid false-negative failures on successful publishes.
# See mika#1917 run 32160868980 — publish step succeeded (@samidarko/ui@0.3.1
# live), verify raced ~2s post-publish, all 5×15s attempts returned empty
```

La boucle 5×15 s **est** le correctif de mika#1917. Elle a été posée sur une
borne supposée (« up to ~60 s »), jamais mesurée, et elle a tenu jusqu'au bump
suivant. **Deuxième occurrence de la même classe, et le remède proposé est le
même que la première fois.** Rallonger à 8×backoff, c'est poser une troisième
borne supposée et attendre le troisième bump.

Trois faits supplémentaires, lus dans le fichier :

- **La boucle dort après le dernier essai.** `sleep` est inconditionnel en fin
  de corps, donc 5 essais coûtent 5 sommeils : le 5ᵉ essai a lieu à `t=60 s`, et
  les 15 dernières secondes sont dormies pour rien avant l'échec. La fenêtre
  annoncée « 75 s » est en réalité **60 s de couverture**.
- **`2>/dev/null || echo ""` efface la cause.** Un `E404` (la version n'est pas
  encore là — attendu pendant la propagation), un `ENETWORK` (le runner n'a pas
  de réseau) et un `E401` (jeton invalide) rendent des bytes strictement
  identiques : la chaîne vide. Le message d'échec dit *« not visible on the
  default registry »* dans les trois cas, et il n'est vrai que dans le premier.
- **La logique est enfouie dans un `run:` de YAML**, donc exercée par rien. Le
  correctif de mika#1917 n'a pas pu être vérifié avant d'atteindre la production,
  et le seul moment où on découvre qu'il ne suffit pas est le bump suivant, sur
  `main`, en rouge. C'est la propriété structurelle à changer — pas le nombre.

---

## Deux hypothèses de cause, et on ne choisit pas

Le commentaire du fichier pose « CDN propagation ». C'est **une** hypothèse, et
une seconde est disponible par lecture du même fichier :

| | hypothèse | mécanisme | ce qui la refermerait |
|---|---|---|---|
| **H1** | propagation registre / CDN | le packument mis à jour met plus de temps que la fenêtre à être servi | allonger la fenêtre |
| **H2** | cache npm **local du runner** | l'étape « Check if version changed » (l. 32) appelle `npm view @samidarko/ui version` **avant** la publication, ce qui met en cache le packument d'**avant**. La vérification relit le même packument, et npm sert une entrée encore fraîche au lieu de revalider | forcer la revalidation |

H2 n'est pas spéculative sur son mécanisme — l'appel pré-publication est à la
ligne 32 du fichier et il vise le même document que la vérification. Ce qui n'est
pas établi, c'est **laquelle des deux produit le délai mesuré**, et ça n'est pas
établissable depuis un worktree : il faut le log d'un run réel.

**Décision : on ferme les deux, et on instrumente pour que la prochaine
occurrence tranche.** Choisir maintenant serait reproduire exactement le geste de
mika#1917 — poser une cause par intuition et calibrer un nombre dessus.

La conséquence est asymétrique et vaut d'être écrite : **si H2 est vraie, une
fenêtre longue la couvre par accident** (le cache expire) sans qu'on sache
pourquoi, et le correctif se relit comme une victoire de l'allongement. Fermer H2
explicitement est ce qui empêche cette lecture fausse.

---

## Décisions

### D1 — La logique sort du YAML vers `scripts/verify-npm-publish.sh`

Générique (`<package> <version>` en arguments, aucun littéral `@samidarko/ui`),
versionné, exerçable en une seconde par un harnais. **C'est le changement qui
rend ce correctif différent du précédent** : le suivant sera vérifié avant de
partir, pas au bump d'après.

### D2 — La géométrie : 8 essais, 15 s doublés, plafond 60 s

Sommeils entre essais uniquement (7 sommeils pour 8 essais) :
`15 + 30 + 60 + 60 + 60 + 60 + 60 = 345 s`. Le 8ᵉ essai a lieu à **t = 345 s**,
soit **5 min 45 s de couverture** — au-delà des « ~5 min » de l'AC1.

**Ce nombre est posé, pas mesuré**, et c'est à dire plutôt qu'à laisser croire :
aucune borne supérieure de propagation npm n'est connue de ce dépôt, et les deux
valeurs antérieures (« ~60 s » de mika#1917, 75 s annoncés / 60 s réels) étaient
également posées. Ce qui change n'est pas la qualité du nombre, c'est que son
insuffisance sera désormais **diagnostiquée** (D4) au lieu d'être re-devinée.

### D3 — `--prefer-online` sur la requête de vérification

Ferme H2 à sa cause. Appliqué à la **vérification seule** : l'étape « Check if
version changed » tourne avant la publication, son résultat est frais par
construction, et y toucher n'apporterait rien.

### D4 — Sur échec seulement, un bloc de diagnostic qui sépare H1 de H2

Le script, dans sa branche d'échec et nulle part ailleurs, émet :

1. le **stderr de la dernière tentative** (la cause que `2>/dev/null` effaçait) ;
2. le résultat d'un `npm view --prefer-online` final ;
3. une requête **HTTP directe** au registre
   (`https://registry.npmjs.org/<pkg-url-encodé>`, scope encodé `/` → `%2F`,
   parsée avec `node -e` — garanti présent, `setup-node` a déjà tourné), disant
   si la version est présente dans le packument servi.

Table de lecture, imprimée par le script lui-même :

| registre HTTP direct | `npm view` | lecture | remède |
|---|---|---|---|
| porte la version | ne la voit pas | **H2** — client/cache npm | côté client, **jamais** la fenêtre |
| ne la porte pas | ne la voit pas | **H1** — propagation réelle | la fenêtre, avec cette mesure |
| injoignable | — | ni H1 ni H2 — le runner n'a pas de réseau | la boucle a tourné 6 min pour rien |

Coût en régime nominal : **nul** (la branche ne tourne pas).

### D5 — Une erreur non-`E404` ne raccourcit pas la boucle

Tentant, et refusé : sortir tôt sur une erreur réseau transformerait un incident
transitoire en faux rouge d'une autre espèce — le défaut de ce ticket, sous un
autre nom. La boucle va au bout ; c'est le **message final** qui nomme la cause.

### D6 — La géométrie est injectable, et elle est dite

`PUBLISH_VERIFY_MAX_ATTEMPTS`, `PUBLISH_VERIFY_SLEEP_BASE_SECS`,
`PUBLISH_VERIFY_SLEEP_CAP_SECS`.

**Ce ne sont pas des réglages d'opérateur — c'est l'injection qui rend le test
déterministe et instantané** au lieu de dormir six minutes. Le workflow n'en pose
aucune. Paliers :

| clé | absent/vide | `0` | négatif / illisible |
|---|---|---|---|
| `MAX_ATTEMPTS` | défaut `8` | **défaut + avertissement** — un `0` désarmerait la vérification, et un désarmement par coquille sur une garde est la panne silencieuse que ce ticket ferme | défaut + avertissement |
| `SLEEP_BASE_SECS` | défaut `15` | **honoré** (le test en a besoin) | défaut + avertissement |
| `SLEEP_CAP_SECS` | défaut `60` | **honoré** | défaut + avertissement |

Levier résiduel nommé : `SLEEP_BASE=0` en production désarmerait l'attente sans
désarmer la vérification. Rien ne pose ces variables sur un runner GitHub, et le
contre-poids est la ligne ci-dessous.

### D7 — La géométrie résolue est journalisée à chaque exécution

```
verify geometry: attempts=8 base=15s cap=60s window=345s prefer-online=yes
```

Doctrine mika#2293 : *un réglage qu'on ne peut pas observer n'est pas un réglage,
c'est un espoir.* Cette ligne est aussi le **contrôle positif** de la sonde S1 :
son absence dans le log d'un run dit que le workflow servi n'appelle pas le
script, jamais que tout va bien.

### D8 — Refusé : sortir la vérification dans un job `continue-on-error`

C'est la seconde branche du « Remède proposé » du ticket, et elle est **refusée
avec sa raison**. Un job dont le rouge ne colore pas le run est une garde
désarmée : elle satisfait AC2 à la lettre (« la vérification échoue ») en
annulant ce pour quoi elle existe. Et c'est littéralement le second effet que le
ticket nomme comme le pire — « on apprend à l'ignorer » — institutionnalisé dans
le YAML. *Une garde qu'on n'a pas armée se lit exactement comme une flotte
saine* (classe mika#2205).

---

## Livrables

| # | fichier | nature |
|---|---|---|
| 1 | `scripts/verify-npm-publish.sh` | **neuf** — D1..D7 |
| 2 | `scripts/test-verify-npm-publish.sh` | **neuf** — le harnais (détecteur) |
| 3 | `.github/workflows/publish-ui.yml` | l'étape appelle le script ; le commentaire mika#1917 est **conservé et daté**, pas effacé |
| 4 | `Makefile` | cible `test-verify-npm-publish` + entrée `.PHONY` |
| 5 | `.github/workflows/ci.yml` | job `publish-verify-lint` (gabarit `shared-checkout-guard-lint`) |

Gabarit de l'étape (3) :

```yaml
      - name: Verify publish (mika#2566 — fenêtre bornée, cause diagnostiquée)
        if: steps.version.outputs.skip != 'true'
        run: |
          bash scripts/verify-npm-publish.sh @samidarko/ui \
            "$(node -p "require('./packages/ui/package.json').version")"
```

Le harnais (2) place un `npm` factice en tête de `PATH` — il journalise son argv
et rend la version au-delà d'un seuil d'appels lu dans un fichier compteur — plus
un `curl` factice pour la branche de diagnostic, et injecte `SLEEP_BASE=0`,
`SLEEP_CAP=0`.

---

## Verification Contract

Neuf assertions, toutes déterministes et hors réseau.

| # | assertion | ce qu'elle tient |
|---|---|---|
| N1 | version visible au 1ᵉʳ essai → `exit 0`, **un seul** appel `npm` | le chemin nominal ne dort pas |
| N2 | version visible au **6ᵉ** essai → `exit 0` | **AC1** — le cas que l'ancienne géométrie (5 essais) ratait |
| N3 | version jamais visible → `exit 1` | **AC2**, l'invariant dur |
| N4 | avec `attempts=3, base=1, cap=1`, la durée totale est `< 3 s` | 2 sommeils pour 3 essais — le sommeil terminal a disparu |
| N5 | l'argv du `npm` factice porte `--prefer-online` | **D3** est réellement branché |
| N6 | `npm` en erreur `ENETWORK` → le message final **contient** `ENETWORK`, et le bloc de diagnostic est émis | **D4** — la cause n'est plus effacée |
| N7 | dans N3, le `npm` factice a été appelé **exactement `MAX_ATTEMPTS` fois** | anti-vacuité : un script trivial l'appellerait 0 fois et N3 serait vert pour la mauvaise raison |
| N8 | la ligne `verify geometry:` est émise et **porte les valeurs injectées** | **D7** — la géométrie dite est celle appliquée |
| N9 | `MAX_ATTEMPTS=0` → 8 essais **et** un avertissement nommant `"0"` | **D6** — pas de désarmement silencieux |

**N7 est le contrôle positif du harnais et n'est pas décoratif.** Sans lui,
remplacer `verify-npm-publish.sh` par `exit 1` rendrait N3 vert — un détecteur
silencieusement inerte se lit exactement comme un détecteur sain.

`npm` n'est jamais appelé pour de vrai ; la suite tourne en moins d'une seconde.

---

## Fire-Disposition

Ce plan livre un détecteur : `scripts/test-verify-npm-publish.sh`, armé en CI par
le job `publish-verify-lint`. Son chemin de succès est « aucune violation
trouvée ».

**Option (a) — exception nommée en allowlist, table LIVRÉE VIDE.**

Il n'existe **aucune violation préexistante à exempter** : `verify-npm-publish.sh`
est neuf, il est la seule chose que la suite exerce, et il est écrit pour passer
les neuf assertions. L'arbre est vert au moment de la livraison, sans une seule
entrée. Le cas dégénéré de (a) est donc l'état livré, et il est **asserté** plutôt
que constaté — patron `scripts/test-guard-shared-checkout.sh` :

```sh
# Forme obligatoire d'une entrée (aucune n'existe aujourd'hui) :
#   "<assertion> | <donnée exacte> | <ticket de suivi> | <condition de péremption>"
VERIFY_EXCEPTION_ALLOWLIST=()
```

Deux assertions auto-nettoyantes ferment la table :

1. `${#VERIFY_EXCEPTION_ALLOWLIST[@]} -eq 0` — la suite rougit le jour où une
   entrée apparaît sans ticket de suivi ni condition de péremption ;
2. `grep -qE 'ALLOWLIST|EXCEPTION'` sur `verify-npm-publish.sh` doit **ne rien
   rendre** — une table consultée au runtime serait une échappatoire dans la
   garde elle-même, sur le chemin exact où AC2 doit tenir sans condition.

**Règle écrite en tête du harnais : quand une assertion tire, on répare
`verify-npm-publish.sh` ; on n'ajoute pas de ligne à la table** (doctrine
mika#2201). Un cas qu'on ne veut pas couvrir est un cas à retirer, pas à
exempter.

**Ni option (b) ni option (c) :** (b) — livrer désarmé — laisserait AC2 sans
assertion, c'est-à-dire livrerait le correctif sans ce qui le distingue de celui
de mika#1917 ; (c) — halte-et-remontée — n'a pas d'objet, l'arbre étant vert.

---

## Definition of Done

- [ ] `scripts/verify-npm-publish.sh` livré, exécutable, générique (`<package>
      <version>`), sans littéral de paquet.
- [ ] Backoff 8 / 15 s / plafond 60 s, **sommeils entre essais uniquement**.
- [ ] `--prefer-online` sur la requête de vérification.
- [ ] Bloc de diagnostic sur échec (stderr de la dernière tentative + `npm view`
      final + registre HTTP direct + table de lecture).
- [ ] Ligne `verify geometry:` émise à chaque exécution.
- [ ] Trois clés injectables aux paliers de D6, avertissement nommant la valeur
      entre guillemets.
- [ ] `scripts/test-verify-npm-publish.sh` : N1–N9 vertes, table d'exceptions
      vide et assertée.
- [ ] `publish-ui.yml` appelle le script ; le commentaire mika#1917 conservé,
      daté, et augmenté de la rectification (« ce retry EST le correctif de
      mika#1917 ; mika#2566 est sa deuxième occurrence »).
- [ ] `Makefile` : cible `test-verify-npm-publish` + `.PHONY`.
- [ ] `ci.yml` : job `publish-verify-lint` sur `ubuntu-22.04`, checkout épinglé
      au même SHA que ses voisins.
- [ ] `make check` et `make lint` verts.

---

## Acceptance criteria

- [ ] **Une publication qui met jusqu'à ~5 min à se propager ne rend plus le run
      rouge.** Tenu par la géométrie de D2 (couverture 345 s) et **asserté
      déterministiquement** par N2 (version visible au 6ᵉ essai → `exit 0`), qui
      est précisément le cas que la géométrie à 5 essais ratait.
- [ ] **Une version qui n'apparaît jamais fait toujours échouer la
      vérification.** Tenu par N3 (`exit 1`), adossé à N7 (le `npm` factice a bien
      été appelé `MAX_ATTEMPTS` fois — sans quoi N3 serait vert pour la mauvaise
      raison), et protégé de toute échappatoire par la seconde assertion de la
      Fire-Disposition (aucune allowlist consultée au runtime). D8 refuse
      explicitement la variante `continue-on-error`, qui satisferait cet AC à la
      lettre en l'annulant en fait.

---

## Sondes post-déploiement, et leurs quatre haltes

> **Préalable, et il n'est pas anodin.** `publish-ui.yml` se déclenche sur
> `push` vers `main` **avec `paths: packages/ui/**`**. Ce merge touche
> `.github/`, `scripts/` et `Makefile` — il **ne déclenche donc pas le workflow
> qu'il corrige**. La sonde est le **prochain bump réel de `packages/ui/`**, pas
> ce merge. Sans cette phrase, on merge, on ne voit rien, et on croit avoir
> vérifié.

**S1 — le chemin nominal (prochain bump).** Le job sort **vert**, et le log porte
la ligne `verify geometry: attempts=8 base=15s cap=60s window=345s
prefer-online=yes` puis `Verified @samidarko/ui@<v> is live … (attempt N/8)`.

**S2 — attribution (même run).** La valeur de `N` **est** la mesure qui manquait
depuis mika#1917 : combien de temps la propagation prend réellement. À noter
quelque part le jour où elle est lue.

**Halte 1 — le job sort rouge.** **Lire le bloc de diagnostic AVANT de toucher à
la fenêtre.** Registre HTTP direct portant la version ⇒ **H2**, et rallonger ne
réglera rien — le remède est côté client npm. Ni l'un ni l'autre ⇒ **H1**, et
c'est alors la **troisième** occurrence de la classe : ouvrir un ticket de suivi
**avec cette mesure**, jamais rallonger au jugé. À ce stade, la fenêtre n'est
plus le bon remède et c'est le diagnostic qui le dira.

**Halte 2 — le job sort vert à `attempt 1/8` systématiquement.** `--prefer-online`
a fermé H2, et la fenêtre longue est une assurance et non un besoin. **C'est un
résultat à écrire**, pas une panne — et c'est ce qui autoriserait, plus tard, à
la raccourcir sur mesure.

**Halte 3 — le job sort vert alors que la publication a échoué.** AC2 est violée.
**Désarmer d'abord** (revert de l'étape vers la forme antérieure), diagnostiquer
ensuite : une vérification qui passe sur une publication ratée est strictement
pire que le faux rouge qu'elle remplace.

**Halte 4 — le job est vert et la ligne `verify geometry:` est absente.** Le
workflow servi n'appelle pas le script (YAML antérieur, ou `paths` non déclenché).
**On ne peut rien conclure** : établir cela avant toute lecture des autres sondes.

---

## Ce que ce travail n'achète pas

- **Il ne fait pas publier ni propager plus vite.** Il borne l'attente et rend
  l'échec lisible.
- **Il ne tranche pas H1 contre H2.** Il ferme les deux et **instrumente** pour
  que la prochaine occurrence tranche — ce que ni mika#1917 ni le présent ticket
  ne pouvaient faire.
- **Aucun compteur, aucune surface d'agrégation, aucun événement.** Le seul
  instrument est le log du run, et **son silence ne prouve rien tant qu'un bump
  n'a pas eu lieu** — le workflow ne tourne que sur `packages/ui/**`, quelques
  fois par mois.
- **Il ne garantit pas que 345 s suffiront.** Il garantit que le jour où ça ne
  suffira pas, on saura pourquoi au lieu de reposer un nombre.

---

## Hors périmètre, délibérément

- **Un scan de source refusant le retour d'une boucle de retry inline dans un
  `run:` de YAML.** Population d'un seul site ; la garde effective est que le
  workflow appelle le script et que le harnais l'exerce. **Suivi** si un second
  site apparaît.
- **Le job `continue-on-error`** — refusé avec sa raison (D8).
- **Le `npm view` de l'étape « Check if version changed »** — inchangé : il
  tourne avant publication, son résultat est frais par construction, et c'est
  `--prefer-online` côté vérification qui neutralise son effet de bord.
- **Une vérification côté CDN de consommation** (`unpkg`, `jsdelivr`) — autre
  population, autre question, aucune mesure ne la demande.
- **La publication elle-même**, le `NPM_TOKEN` et la stratégie de versionnement
  de `packages/ui/`.
- **Généraliser le script aux autres workflows** — un seul publie un paquet npm ;
  dessiner une abstraction sur un point est ce qui produit la mauvaise
  abstraction.
