---
title: "Une boucle de retry calibrée sur une borne supposée revient : l'extraire, l'exercer, et faire nommer la cause par son échec"
date: 2026-09-29
category: ci-cd
module: ci-cd
component: publish-ui
tags: [github-actions, npm, publish, retry, backoff, propagation, prefer-online, anti-vacuity, negative-control, mika-2566, mika-1917]
problem_type: integration_issue
symptoms:
  - "publish-ui sort rouge sur main (« Post-publish verification failed after 5 attempts ») alors que la version est bel et bien publiée"
  - "npm view rend la version quelques minutes après l'échec du run"
  - "le message d'échec dit « not visible on the default registry » quelle que soit la cause réelle"
root_cause: async_timing
resolution_type: tooling_addition
severity: low
issue: "mika#2566"
---

# Une boucle de retry calibrée sur une borne supposée revient : l'extraire, l'exercer, et faire nommer la cause par son échec

## Problème

Run 36410711981 sur `main`, 2026-09-28 : après le bump `@samidarko/ui` 0.3.1 → 0.4.0, l'étape de vérification post-publish de `publish-ui.yml` a épuisé ses cinq essais et rendu le run rouge. Quelques minutes plus tard, `npm view @samidarko/ui version` rendait `0.4.0` : la publication avait réussi. Faux rouge sur `main`.

Le ticket proposait d'allonger la fenêtre. **Cette boucle 5×15 s était déjà le correctif de mika#1917** (commit `d32a0fd7`, « publish-ui verify step retry+backoff for CDN propagation lag »), posé sur une borne jamais mesurée — « npmjs CDN can take up to ~60s ». mika#2566 est la deuxième occurrence de la même classe, et le remède proposé était celui qui avait déjà échoué.

## Symptômes

- Rouge sur `main` au bump d'une version publiée avec succès.
- `Attempt 1/5` … `Attempt 5/5` puis `Post-publish verification failed after 5 attempts`.
- Aucun indice dans le log sur la cause : un 404 de propagation, une panne réseau du runner et un jeton invalide produisent le même message.

## Ce qui n'a pas marché

- **Allonger la fenêtre, la première fois (mika#1917).** Le nombre a tenu jusqu'au bump suivant. Comme la logique vivait dans un `run:` de YAML, rien ne l'exerçait : son insuffisance ne pouvait se découvrir qu'en production, au bump d'après, sur `main`.
- **Allonger la fenêtre, la seconde fois (le remède du ticket).** Refusé : c'est poser une troisième borne supposée et attendre le troisième bump.
- **Sortir la vérification dans un job `continue-on-error`** (seconde branche du remède proposé). Refusé : un job dont le rouge ne colore pas le run est une garde désarmée. Il satisfait « la vérification échoue » à la lettre en annulant ce pour quoi elle existe.

## Trois défauts lisibles dans l'ancienne boucle

Tous visibles à la lecture de l'ancien `run:` (retiré par ce correctif, voir le diff de `.github/workflows/publish-ui.yml` sur PR mika#2581) :

1. **Le `sleep` était inconditionnel en fin de corps de boucle.** 5 essais coûtaient 5 sommeils : le 5ᵉ essai avait lieu à t = 60 s, puis 15 s dormies pour rien avant l'échec. Les « 75 s » annoncés couvraient en réalité **60 s**.
2. **`npm view … 2>/dev/null || echo ""` effaçait la cause.** `E404` (attendu pendant la propagation), `ENETWORK` et `E401` rendaient tous la chaîne vide, et le message final affirmait « not visible on the default registry » dans les trois cas.
3. **Deux hypothèses de cause, dont une ne se règle pas par la fenêtre.**
   - **H1** — propagation registre/CDN réelle : la fenêtre est le remède.
   - **H2** — cache npm **local** du runner : l'étape « Check if version changed » appelle `npm view @samidarko/ui version` *avant* la publication, ce qui met en cache le packument d'avant. La vérification relit ce même document, et npm peut le resservir sans revalider. Rallonger ne règle alors rien. Au mieux, le cache expire par accident et le correctif se relit à tort comme une victoire de l'allongement.

   Laquelle a produit le délai mesuré ne s'établit pas depuis un worktree ; il faut le log d'un run réel.

## Solution

La logique sort du YAML vers `scripts/verify-npm-publish.sh <package> <version>` (générique, sans littéral de paquet), exercée hors réseau par `scripts/test-verify-npm-publish.sh`, armée à chaque PR par le job CI `publish-verify-lint`. L'étape du workflow se réduit à :

```yaml
      - name: Verify publish (mika#2566 — fenêtre bornée, cause diagnostiquée)
        if: steps.version.outputs.skip != 'true'
        run: |
          bash scripts/verify-npm-publish.sh @samidarko/ui \
            "$(node -p "require('./packages/ui/package.json').version")"
```

Le script :

- **Ferme les deux hypothèses au lieu d'en choisir une.** Il utilise 8 essais, un backoff doublé depuis 15 s et plafonné à 60 s, avec des sommeils *entre* essais uniquement (`scripts/verify-npm-publish.sh:296` : `if [ "$attempt" -lt "$MAX_ATTEMPTS" ]`), soit 345 s de couverture pour H1. Il passe `--prefer-online` sur la requête de vérification (`:283`) pour H2.
- **Garde le stderr de chaque tentative** et, **sur échec seulement**, émet un bloc de diagnostic : stderr de la dernière tentative, un `npm view --prefer-online` final, puis une requête HTTP directe au packument (`https://registry.npmjs.org/<scope%2Fname>`, parsée avec `node -e`). Une table de lecture imprimée par le script dit laquelle des deux hypothèses le run vient de démontrer (`:228-230`). Si le registre direct porte la version et que npm ne la voit pas, c'est H2 : le remède est côté client, jamais la fenêtre. Si ni l'un ni l'autre ne la voit, c'est H1 : la fenêtre, avec cette mesure. Si le registre est injoignable, ce n'est ni H1 ni H2 : le runner n'avait pas de réseau.
- **Ne raccourcit pas la boucle sur une erreur non-`E404`** : sortir tôt sur une erreur réseau fabriquerait un faux rouge d'une autre espèce. C'est le message final qui nomme la cause.
- **Journalise sa géométrie résolue à chaque exécution** (`:265`) :
  `verify geometry: attempts=8 base=15s cap=60s window=345s prefer-online=yes`.
  Cette ligne est aussi le contrôle positif du déploiement : son absence dans le log d'un run dit que le workflow servi n'appelle pas le script.
- **Rend la géométrie injectable** (`PUBLISH_VERIFY_MAX_ATTEMPTS`, `…_SLEEP_BASE_SECS`, `…_SLEEP_CAP_SECS`) pour que le harnais tourne en moins d'une seconde. Ce ne sont pas des réglages d'opérateur. `MAX_ATTEMPTS=0` est refusé avec un avertissement, parce qu'il désarmerait la vérification par une coquille. Les durées, elles, honorent `0`.

## Pourquoi ça marche

Ce qui change n'est pas le nombre d'essais : 345 s restent une borne **posée, pas mesurée**, exactement comme les 60 s de mika#1917. Ce qui change, c'est que :

1. la logique est **exercée avant de partir** (le job `publish-verify-lint` passe sur la PR mika#2581) au lieu d'être découverte insuffisante au bump suivant ;
2. le jour où 345 s ne suffiront pas, le run **dira pourquoi** (H1, H2 ou pas de réseau) au lieu d'inviter à reposer un nombre au jugé.

## Prévention

- **Un retry calibré sur une borne externe non mesurée est une hypothèse, pas un correctif.** S'il revient, ne pas rallonger : faire en sorte que son échec mesure et nomme la cause.
- **Une logique non triviale dans un `run:` de YAML n'est exercée par rien.** Surtout quand le workflow porte un filtre `paths:` qui le fait tourner rarement : ici, `publish-ui.yml` ne se déclenche que sur `packages/ui/**`, quelques fois par mois. L'extraire dans un script avec un harnais et un job CI qui tourne sur chaque PR.
- **Corollaire du filtre `paths:`** : le merge de ce correctif (qui touche `.github/`, `scripts/`, `Makefile`) **ne déclenche pas le workflow qu'il corrige**. La vraie sonde est le prochain bump de `packages/ui/`. Si la ligne `verify geometry:` manque dans ce run, on ne peut rien conclure du vert.
- **`2>/dev/null || echo ""` sur une commande de vérification confond les causes.** Capturer stderr et le restituer au moins sur la branche d'échec.
- **Une boucle `for … do <essai>; sleep; done` dort après le dernier essai.** Conditionner le sommeil à `attempt < max`, et calculer la fenêtre annoncée comme la somme des sommeils *entre* essais.
- **Un harnais de garde doit avoir un contrôle anti-vacuité.** Remplacer le script par `exit 1` laisse vert l'assertion « une version jamais visible fait échouer » (N3). Seule N7, qui exige que le `npm` factice ait été appelé exactement `MAX_ATTEMPTS` fois, la fait rougir. Les mutations jouées sont consignées en tête de `scripts/test-verify-npm-publish.sh`, et le harnais accepte en argument un script mutilé pour les rejouer.
- **Le harnais dépend de `node` pour la branche de diagnostic** (N6b). Il est présent sur les runners GitHub. Sur un poste où `node` n'est pas dans le `PATH` (nvm non chargé), N6b rougit sans que le script soit en cause : charger nvm avant de conclure.

## État au moment de l'écriture

Correctif ouvert en PR mika#2581 (draft, non mergée au 2026-09-29). La PR touche `.github/workflows/`, donc le merge revient à l'opérateur. H1 contre H2 reste non tranché. Au premier bump réel qui suivra le merge, lire `attempt N/8` dans la ligne `Verified …` : c'est la mesure de propagation qui manquait depuis mika#1917.

## Voir aussi

- `docs/plans/2026-09-28-002-fix-2566-publish-ui-verify-testable-et-borne-plan.md` — le plan, ses décisions D1–D8 et les quatre haltes post-déploiement.
- `docs/solutions/test-failures/bash-assert-sigpipe-and-host-coupling-before-ci-gate-2026-08-29.md` — autre cas où un harnais bash couplé à l'hôte rougit hors CI.
