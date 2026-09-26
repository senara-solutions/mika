# mika#2519 — L'exemption existe déjà en double ; ce qui manque est la moitié structurelle et la garde de substance

- **Ticket :** senara-solutions/mika#2519
- **Branche :** `fix/2519/qa-block-pipeline-dependabot-no-plan`
- **Type :** fix
- **Base lue :** HEAD `6b034695` (2026-09-26)

---

## 1. Ce que la lecture du code déplace dans le ticket — premier livrable

Le ticket demande de **livrer** une exemption nommée pour `dependabot[bot]` (AC1)
et de déclarer son label (AC4). Quatre mesures prises sur l'arbre déplacent ce
remède, et chacune change ce qu'il faut écrire.

### R1 — L'exemption de plan existe DÉJÀ, à deux étages, et les deux précèdent la mesure

| étage | site | mergé |
|---|---|---|
| garde exécutable | `scripts/verify-pipeline.sh` mécanisme 4, `AUTOMATED_PR_AUTHORS=("dependabot[bot]" "app/dependabot")` — exempte **les deux** contrôles de buckets sur `.pull_request.user.login` | **2026-09-21** (mika#2419, `93931d40`) |
| prompt QA | `qa-review/system_prompt.md` Step 1.6 — détecte l'auteur dependabot, *« Skip Step 2 pipeline checks and Step 2.5 plan-AC verification »*, *« Do NOT run Steps 2/2.5/3e for a Dependabot PR »* | **2026-08-26** (mika#1729, `a0598967`) |

Step 2B pose déjà `user.login` dans le `GITHUB_EVENT_PATH` synthétique, et
l'avertit mot pour mot : *« omit it and the guard resolves an empty login,
grants no exemption, and every Dependabot Cargo.lock-only PR goes back to
`block[pipeline]` »*.

Le symptôme est mesuré le **2026-09-24**, soit trois jours après le second et un
mois après le premier. **Livrer une troisième couche de la même exemption
construirait sur un diagnostic non établi** — et rejouerait la classe que
mika#2172 a fermée sur ce prompt précis : une règle posée à un troisième endroit
dérive des deux autres.

### R2 — AC4 est déjà satisfait

`pipeline-exempt` est déclaré à `.github/labels.yml:118`. Aucune ligne à écrire.
(La vigilance que l'AC exprime reste juste — `delete-other-labels: true` — elle
est simplement déjà honorée.)

### R3 — Le motif cité n'est le texte d'AUCUNE garde : c'est une prose fabriquée

```
grep -rn "no plan document" --include='*.md' --include='*.sh' --include='*.rs' .
→ zéro ligne
```

La sortie réelle de `verify-pipeline.sh` est
`[pipeline-exempt: none] REJECT: code-only PR: source changes present but no plan/solution doc`.
Le motif rapporté — *« Dependabot dependency bump — no plan document or
`Pipeline-Exempt` trailer present. Build verified successfully. »* — ne provient
d'aucune garde et d'aucun prompt à HEAD. Or Step 2E **exige** qu'un
`block[pipeline]` cite la sortie de sa garde verbatim.

C'est la signature exacte de mika#2237, mesurée sur ce même agent : le skill
mappait `pass → --approve`, la **mémoire** de mika-qa a gagné, argv `--comment`,
zéro tentative. Et c'est la doctrine que le dépôt a écrite
(`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` : neuf
récurrences sous enforcement de prompt contre zéro quand écrit à la main).
**Le prompt exprime l'intention ; il ne tient pas seul au substrat de la boucle.**

### R4 — Le ticket se trompe sur #2454, et cette rectification INVERSE la valeur d'AC2

Le ticket écrit : *« #2454 (jsonwebtoken 9→11, build-vérifié PASS) »* et
*« sur #2453 comme #2454 le build a réussi et aucun site d'appel n'était
cassé »*. L'historique du dépôt dit l'inverse, deux commits plus tard :

> **mika#2525** (`d4514180`, 2026-09-25) — *« Le bump 9.3.1 -> 11.1.0 faisait
> PANIQUER `generate_jwt` : jsonwebtoken 11 a retiré ring pour une architecture
> à provider enfichable et son `default` ne porte que `use_pem`, donc aucun
> backend crypto n'était actif. **Build vert, test rouge.** »*

Trois sites de production appellent `EncodingKey::from_rsa_pem`
(`github_app.rs:95`, `:131`, `doctor.rs:536`), et mika#2523 (`621f12c2`) a dû
réparer un désalignement de version dans la foulée.

Conséquences, dans l'ordre du coût :

1. **« build-vérifié PASS » est vrai et sans valeur.** Le build ne voit pas cette
   classe : le défaut est un `panic!` derrière une cascade `cfg`.
2. **AC2 pose le build comme le filet de l'exemption. La mesure montre que ce
   filet ne tient pas pour la classe majeure.** AC2 reste juste comme *plancher*
   — il n'est pas une garantie.
3. **Le `block` sur #2454 était, par accident, le bon verdict.** Un rail
   dependabot autonome appliqué à cette PR aurait mergé un bump qui panique à la
   génération du JWT GitHub App, c'est-à-dire à l'authentification de toute la
   boucle.
4. **AC3 est donc la seule AC qui portait le risque réel**, et elle est renforcée
   plutôt qu'assouplie.

### Ce que le défaut est, une fois les quatre rectifications posées

Deux trous distincts, de natures différentes :

- **T1 — structurel.** Rien n'empêche mika-qa de poster un `block[pipeline]` sur
  une PR dependabot, verdict que Step 1.6 rend *structurellement inatteignable*.
  L'intention est écrite deux fois ; aucune moitié ne la tient.
- **T2 — substance.** Ni le build, ni la requête d'advisories, ni le scan de
  changelog ne constituent une vérification des **sites d'appel** sur un saut de
  majeure. Step 1.6 ne nomme pas le saut de majeure en tant que discriminant.

---

## 2. Requirements

- **U1** — Établir la cause de T1 avant de livrer quoi que ce soit qui suppose
  une cause (§ 6, sondes S0). Les trois lectures possibles ont trois remèdes
  différents et un seul est du code.
- **U2** — Une garde **pré-subprocess** refuse un `block[pipeline]` posté sur une
  PR dont l'auteur est automatisé, et le dit.
- **U3** — Une garde **pré-subprocess** refuse un `pass` posté sur une PR
  dependabot portant un **saut de majeure** dont le corps du verdict ne porte pas
  de vérification des sites d'appel.
- **U4** — Step 1.6 nomme le saut de majeure comme discriminant, et corrige son
  lecture d'`author` (objet, pas chaîne — § 3.4).
- **U5** — Aucune troisième copie de l'exemption de plan n'est écrite (R1).
- **U6** — Chaque terme illisible **sort la PR de la population** ; une garde
  qu'on n'a pas pu évaluer ne refuse rien.
- **U7** — Les scans de source qui tiennent les deux gardes portent une allowlist
  livrée vide et une assertion auto-nettoyante.

---

## 3. Conception

### 3.1 Le site : la chaîne pré-subprocess de `run_gh`

`crates/mika-agent/src/skills/builtin_handlers.rs::run_gh` porte déjà six gardes
ordonnées « du plus local au plus engageant » (l. 3383–3525) :

```
validate_qa_review_gh_scope          (mika#1196)
validate_pr_ready_undraft_scope      (mika#1682)
validate_destructive_action_grounding(mika#1646)
validate_gh_api_scope                (mika#1167)
validate_review_depth_present        (mika#275)
validate_pr_review_flag_coherence    (mika#2237)
validate_qa_ci_coherence             (mika#2455)   ← dernier, seul appel réseau
```

**Pré-subprocess et non EndTurn, pour la raison que mika#2237 a déjà écrite :**
*« the defect is the call, not a sentence — by the time an EndTurn arm ran, the
review would be on GitHub. »* Un `block[pipeline]` posté est un ticket sorti du
rail autonome ; le rattraper après coup ne le remet pas dessus.

**Placement : immédiatement après `validate_qa_ci_coherence`**, en queue de
chaîne, parce que c'est le second maillon à faire un appel réseau et que l'ordre
existant est justifié par ce critère.

### 3.2 Une garde, deux branches, un seul appel réseau

`validate_dependabot_verdict_coherence(args, repo, ctx)`, avec l'injection de
lecteur du motif mika#2455 (`…_with_reader`, seam sur le **stdout brut** pour que
le test exerce le vrai parseur).

**Reconnaissance gratuite d'abord** — aucun octet de réseau n'est dépensé hors
population :

1. Kill-switch (`MIKA_DEPENDABOT_VERDICT_GATE`) → `Ok(())`.
2. Pas de corps `pr review` extractible → `Ok(())`.
3. `parse_verdict(&body)` ∉ {`BlockPipeline`, `Pass`} → `Ok(())`. Tout autre
   verdict classé (`hold[review]`, `block[ac]`, `block[ci]`,
   `block[dependency]`, `block[security]`) et un corps sans ligne de verdict
   passent intacts.

**Puis un seul appel**, borné par `DEPENDABOT_READ_TIMEOUT_SECS` (10 s, la valeur
de mika#2455) :

```
gh pr view <n> --repo <repo> --json author,title
```

Un appel pour les deux branches. Deux appels doubleraient le coût du maillon que
mika#2455 a placé en dernier précisément parce qu'il en fait un.

**Puis deux classifications pures**, dans `evidence::guards` :

| branche | prédicat | disposition |
|---|---|---|
| **B1** (T1) | verdict `block[pipeline]` **et** `author.login` ∈ `AUTOMATED_PR_AUTHORS` | **refus** — le corps du refus nomme Step 1.6, dit que ce verdict est inatteignable sur cette classe, et nomme les deux issues correctes : rejouer Step 1.6 (`DEP-REVIEW:`), ou — si une garde a réellement échoué — citer sa sortie verbatim (Step 2E) |
| **B2** (T2) | verdict `pass` **et** auteur automatisé **et** saut de majeure lisible dans le titre **et** corps sans ligne `API-SURFACE:` | **refus** — le corps nomme le paquet, le delta, et les deux voies correctes : vérifier les sites d'appel et poser la ligne, ou rendre `block[dependency]` |

`AUTOMATED_PR_AUTHORS` est **importé** de la liste qu'emploie déjà
`verify-pipeline.sh`, jamais recopié : une seconde liste divergerait, et c'est la
classe que mika#2205 a mesurée (un accesseur étroit à côté du résolveur
canonique). La liste vivant aujourd'hui dans un script shell, le plan la porte
côté Rust dans `evidence::guards` **avec un scan de parité bidirectionnel** sur
le tableau shell — la seule forme qui rougit quand l'une des deux bouge.

**Égalité exacte, jamais sous-chaîne** : un auteur nommé `not-dependabot[bot]` ne
doit pas apparier — la contrainte que `verify-pipeline.sh` écrit déjà.

### 3.3 Le saut de majeure : détection permissive, décision stricte

Titres réellement émis par Dependabot :

- `Bump <pkg> from <old> to <new>` → un couple.
- `Bump the <group> group with N updates` → **aucun couple lisible dans le
  titre**. La branche B2 **abstient** : le corps porte la table, et lire une
  table markdown produite par un tiers dans un prédicat de refus est la
  fragilité que ce plan refuse. Coût nommé : un groupe portant un saut de
  majeure n'est pas couvert (§ 8).

Extraction de majeure : premier segment numérique de la version, `0` inclus.
`0.22 → 0.23` **n'est pas** un saut de majeure au sens sémantique strict
(`major == 0` des deux côtés), ce qui est correct pour #2453 — la PR témoin dont
le ticket dit qu'elle devait passer. `9 → 11` en est un.

> **Décision nommée :** la règle `0.x` de semver (où la mineure porte les
> ruptures) n'est **pas** appliquée. L'élargir ferait entrer `0.22 → 0.23`, donc
> #2453 et #2300/#2301/#2302 — quatre des cinq PR témoins — dans la population
> de B2, c'est-à-dire refuser le rail que ce ticket existe pour ouvrir. Un saut
> de mineure sur `0.x` reste couvert par le scan de changelog de Step 1.6
> (`block[dependency]`). Si une mesure montre une rupture `0.x` passée au
> travers, c'est **un ticket avec son compte**, pas un élargissement au jugé.

Version non parsable, titre absent, titre hors des deux formes → **abstention
nommée**, jamais un refus.

`API-SURFACE:` est lu **permissivement** (préfixe ancré en début de ligne, casse
repliée, pas d'ancre de fin — la doctrine de `verify-pipeline.sh` sur son propre
motif de section AC) et **décidé strictement** (absent ou sans contenu ⇒ refus).
C'est un jeton de fil : il reçoit sa ligne dans `scripts/canonical-tokens.tsv`
avec sa tolérance (`ci:line-anchored`) et son site de match — sans quoi
`canonical-tokens-lint` ne le voit pas et la classe mika#2201 se rouvre.

### 3.4 Step 1.6 — deux corrections, et l'une est sur le seul discriminant du chemin

**(a) `author` est un objet, pas une chaîne.** `qa_pr_view.sh` expose
`author` dans ses `SAFE_FIELDS`, et `gh pr view --json author` rend
`{"id":…,"is_bot":true,"login":"app/dependabot","name":""}`. Step 1.6 écrit
*« Read the `author` field … If `author == "dependabot[bot]"` »* — une
comparaison objet↔chaîne sur **le seul discriminant de tout le chemin
dependabot**. Corrigé en `author.login`, avec les deux formes conservées
(`gh` rend l'une ou l'autre selon la surface).

**(b) Le saut de majeure devient un discriminant nommé.** Step 1.6 point 4
raisonne sur le changelog, point 5 sur les advisories ; aucun des deux ne dit
« la majeure a changé, va lire les sites d'appel ». La clause ajoutée l'exige et
nomme la ligne `API-SURFACE:` que B2 lit. **La moitié qui tient est B2, pas la
clause** — c'est la raison d'être de ce plan, et l'écrire à l'envers serait
reproduire T1.

### 3.5 Ce qui n'est PAS écrit

Aucune ligne d'exemption de plan nouvelle (U5) : ni dans `verify-pipeline.sh`,
ni dans `.github/labels.yml`, ni dans `ci.yml`, ni une troisième mention dans le
prompt. Les deux étages de R1 sont laissés intacts, et les sondes S0/S1
établissent lequel n'a pas atteint l'agent.

---

## 4. Fire-Disposition

Ce plan livre des détecteurs : deux branches de garde pré-subprocess, un scan de
parité shell↔Rust, deux scans de prompt, une ligne de lint de jeton.

**Option retenue : (a) exception nommée en allowlist — allowlist livrée VIDE.**

Justification par détecteur, et le raisonnement diffère selon la temporalité de
la population :

| détecteur | population existante | disposition |
|---|---|---|
| B1, B2 (garde pré-subprocess) | **aucune**. La garde s'interpose sur des appels **futurs** ; les cinq verdicts mesurés sont déjà postés et hors de sa portée. Il n'y a rien à exempter. | armée, sans allowlist — une allowlist y serait une exemption sans population |
| scan de parité `AUTOMATED_PR_AUTHORS` | à mesurer à l'implémentation. Les deux listes sont attendues identiques (deux entrées). | allowlist **vide**, comparaison **bidirectionnelle** : une entrée qui ne matche plus rien rougit |
| scans de prompt (`author.login`, clause majeure) | l'arbre est le sujet même du correctif : les scans rougissent **avant** l'édition et passent **après**, par construction | allowlist **vide** |
| ligne `canonical-tokens.tsv` | le jeton `API-SURFACE:` est neuf, aucune occurrence préexistante | pas d'entrée d'exception (`scripts/canonical-tokens-exceptions.tsv` reste vide) |

**Geste si un scan rougit à l'implémentation** — et l'ordre est le livrable :
d'abord **armer le site**, jamais l'allowlister (doctrine mika#2201 : *« on
déclare, on n'allowliste pas »*). Une entrée d'exception n'est écrite que si un
site refuse structurellement d'être armé ; elle nomme alors la donnée précise,
référence son ticket de suivi, et porte son assertion auto-nettoyante.

**Contrôle négatif obligatoire, vu rouge avant d'être livré vert** — sans lui,
« la garde décide » est indistinguable de « la garde refuse tout » :

- B1 sur un auteur **humain** avec `block[pipeline]` → passe.
- B1 sur un auteur automatisé avec `hold[review]` → passe.
- B2 sur `0.22 → 0.23` (#2453) → passe.
- B2 sur `9 → 11` **avec** `API-SURFACE:` renseignée → passe.
- Le scan de parité avec les deux listes alignées → vert ; une entrée retirée
  d'un côté → rouge.

---

## 5. Verification contract

**V1 — prédicats purs** (`evidence::guards::tests::mika2519`) : les deux
branches dans les deux sens ; les cinq contrôles négatifs du § 4 ; chaque terme
illisible rendant une abstention **nommée** et jamais un refus ; égalité exacte
d'auteur (`not-dependabot[bot]` ne matche pas) ; extraction de majeure sur
`0.22→0.23`, `9→11`, `1.2.3→2.0.0`, un titre de groupe, un titre hors forme.

**V2 — chemin de production** (`crates/mika-agent/tests/eval/`, `MockLlmProvider`,
sans réseau) : le refus se produit **avant** le sous-processus — aucun
`pr review` n'est posté ; les contrôles négatifs passent ; une ligne
`audit_events` distingue « accepté par le prédicat » de « accepté par
abstention ». Sans cette dernière assertion, une garde devenue inerte se lit
exactement comme une garde saine (classe mika#2205).

**V3 — réseau indisponible** : lecteur en timeout, en échec, sortie non
parsable, token absent, cible non numérique, dépôt absent → **abstention**, le
`pr review` passe. Une garde qu'on n'a pas pu évaluer ne refuse rien (U6).

**V4 — parité de la liste d'auteurs**, bidirectionnelle, allowlist vide.

**V5 — scans de prompt**, hébergés dans
`crates/mika-agent/tests/qa_review_executes_repo_guards.rs` (déjà propriétaire
de `step_2b_synthetic_event_carries_the_pr_author`, mika#2419) : Step 1.6 lit
`author.login` et non `author` nu ; Step 1.6 nomme le saut de majeure et la ligne
`API-SURFACE:`. Plus une **assertion d'anti-vacuité** : le scan échoue si le
prompt ne contient pas Step 1.6 du tout — un scan qui vise une section disparue
se lit exactement comme un arbre propre.

**V6 — `canonical-tokens-lint` vert** avec la ligne `API-SURFACE:` déclarée, et
`scripts/canonical-tokens-survey.sh --check` propre.

**V7 — non-régression** : `make verify-bundled-skills`, `cargo clippy`,
`cargo fmt --check`, `cargo test -p mika-agent`, et
`scripts/verify-pipeline-test.sh` inchangé et vert (aucune ligne de
`verify-pipeline.sh` n'est touchée).

**Ce qui n'est PAS testable ici, écrit plutôt que découvert :** qu'un verdict
réel de mika-qa change. Le verdict est produit par un LLM contre un vrai dépôt.
Le contrat côté moteur est *un verdict de cette forme ne peut pas être posté*, et
V2 l'atteste déterministement. La moitié comportementale est la sonde S2.

---

## 6. Sondes post-déploiement, et leurs haltes

> **Préalable, avant toute sonde.** `skills/bundled/` est une projection du
> **binaire**, pas du checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer`
> doit porter le sha qu'on vient de bâtir — **sans cette vérification, chaque
> sonde ci-dessous décrit le binaire d'hier.**

### S0 — établir T1 AVANT de conclure quoi que ce soit (U1)

Les trois lectures et leur discriminant :

| lecture | discriminant | remède |
|---|---|---|
| **L1** — le binaire servi le 2026-09-24 précède mika#2419 (2026-09-21) | `.manifest-writer` de l'époque ; les cinq PR n'ont **pas** de section `DEP-REVIEW:` | déploiement, pas code — mais T1 reste ouvert pour la prochaine fois |
| **L2** — Step 1.6 n'a pas déclenché | les verdicts ne portent **ni** `DEP-REVIEW:` **ni** `PIPELINE:` avec sortie de garde | § 3.4(a) : l'imprécision `author` vs `author.login` |
| **L3** — le prompt tient, le modèle ne le suit pas | motif fabriqué, aucune sortie de garde citée (**R3 : c'est ce que la mesure montre**) | B1 — la seule moitié qui tient |

Lecture : `gh pr view 2453 --json reviews --jq '.reviews[].body'` sur les cinq PR
témoins (#2453, #2454, #2302, #2301, #2300). **Halte S0 — si les verdicts citent
une sortie de garde verbatim**, alors la garde a réellement échoué et le
diagnostic entier est faux : lire *quelle* garde et *pourquoi*, **ne pas armer
B1**, qui masquerait un refus fondé.

### S1 — B1 mord (30 jours)

```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'dependabot_verdict_guard' GROUP BY 1;
```
```bash
grep dependabot_verdict_refused "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{repo, pr, verdict, author, reason}'
```

**Régime attendu : non vide et faible.** Chaque ligne est un ticket que le rail
autonome garde. **Halte S1 — un flot soutenu** ne veut pas dire que le prédicat
est trop large : il veut dire que la moitié intention n'atteint pas ce chemin.
**Vérifier le seed du prompt avant de toucher au prédicat** (mika#2340), puis
lire si la mémoire de l'agent re-pousse le verdict retiré — c'est alors le
tagging des mémoires défensives qui s'ouvre, **avec un compte** (mika#2237).

### S2 — le symptôme cesse (le prochain bump Cargo-only, majeure inchangée)

Une PR dependabot `0.x → 0.x` build-verte reçoit un verdict portant
`DEP-REVIEW:`, jamais `block[pipeline]`. **Halte S2 — `block[pipeline]`
réapparaît alors que S1 est vide :** la garde ne voit pas ce chemin. Établir si
le verdict est posté par une autre surface que `run_gh pr review` **avant**
d'élargir le prédicat.

### S3 — B2 mord sur la classe #2454 (le prochain saut de majeure)

Un bump de majeure reçoit soit un `pass` portant `API-SURFACE:`, soit
`block[dependency]`. **Halte S3 — un `pass` sans `API-SURFACE:` franchit :**
désarmer (`MIKA_DEPENDABOT_VERDICT_GATE=0`) **avant** diagnostic — un bump de
majeure mergé sans vérification de surface d'API est ce que #2454 a coûté, et le
coût était l'authentification GitHub App de toute la boucle.

### S4 — contrôle négatif de bruit (7 jours)

Aucun refus sur une PR humaine, aucun refus sur un `0.x → 0.x`.
**Halte S4 — un faux positif :** désarmer d'abord, réparer le prédicat ensuite.
Un faux refus coûte un verdict légitime bloqué ; c'est un arbitrage de prédicat,
pas un seuil à régler.

### S5 — contrôle POSITIF : la garde tourne-t-elle seulement ?

```sql
SELECT count(*) FROM audit_events WHERE tool_name = 'dependabot_verdict_guard';
```

Zéro refus **avec** un compte non nul (abstentions incluses) est un rail sain.
Zéro des deux ne prouve **rien** : la garde peut n'avoir jamais été atteinte.
*Une garde qu'on n'a pas déployée se lit exactement comme une flotte saine*
(mika#2205).

---

## 7. Definition of Done

- [ ] S0 exécutée et sa lecture **écrite sur le ticket** avant merge — le plan
      refuse de construire sur une cause supposée (U1).
- [ ] `validate_dependabot_verdict_coherence` en place après
      `validate_qa_ci_coherence`, avec injection de lecteur, kill-switch,
      abstentions nommées, audit warn-and-continue.
- [ ] Les deux classifieurs purs dans `evidence::guards`, avec leurs constantes
      de fil.
- [ ] Step 1.6 corrigé sur `author.login` et portant la clause de majeure +
      `API-SURFACE:`.
- [ ] `API-SURFACE:` déclaré dans `scripts/canonical-tokens.tsv`.
- [ ] `## Fire-Disposition` honorée : allowlists **vides**, parité
      bidirectionnelle, anti-vacuité sur le scan de prompt, cinq contrôles
      négatifs **vus rouges** avant d'être livrés verts.
- [ ] V1–V7 vertes. `verify-pipeline.sh` **non modifié** ;
      `verify-pipeline-test.sh` vert sans édition.
- [ ] Aucune troisième copie de l'exemption de plan (U5) ; `.github/labels.yml`
      inchangé (R2).
- [ ] Entrée `docs/solutions/` : *« une exemption écrite deux fois et tenue zéro
      fois »*, avec R3 et R4 comme mesures.
- [ ] Corps de PR nommant les quatre rectifications, le statut de chaque AC du
      ticket, et les tickets de suivi du § 8.

---

## Acceptance criteria

**Transcrites verbatim du ticket, avec leur statut établi par lecture du code :**

- **AC1** — Exemption **nommée** pour `author == dependabot[bot]` : soit un
  trailer `Pipeline-Exempt: deps` posé par un bot sur le corps de PR, soit un
  label `pipeline-exempt` appliqué automatiquement sur `author=dependabot`. La
  vérification de plan/AC est alors sautée pour cette classe.
  → **DÉJÀ LIVRÉ, deux fois** (R1) : `verify-pipeline.sh` mécanisme 4
  (mika#2419) et Step 1.6 (mika#1729). Une troisième forme n'est pas écrite
  (U5) ; ce qui est livré est la moitié qui la **tient** (U2). Vérifié par S0/S2.
- **AC2** — **Build-vérif obligatoire** : l'exemption ne dispense PAS du
  build+tests. Un bump qui casse le build reste `block`.
  → **Inchangé et non affaibli** : `qa_build_callback` et la CI restent en place ;
  aucune ligne de ce plan ne les touche. **Rectification R4 : le build était vert
  sur #2454 et le bump était cassé** — AC2 est un plancher, pas une garantie, et
  c'est U3 qui porte la garantie.
- **AC3** — **Jamais un contournement de la QA de substance pour les bumps de
  MAJEURE** : un saut de version majeure (p. ex. jsonwebtoken 9→11) exige une
  vérification des sites d'appel (rupture d'API) au-delà du build — l'exemption
  couvre le *plan*, pas la *substance* d'un changement d'API. Discriminant :
  version majeure changée ⇒ note explicite / revue de substance requise.
  → **Livré, et c'est le cœur du plan** : U3/U4, branche B2, ligne
  `API-SURFACE:`. Renforcée par R4 : #2454 démontre que cette AC était la seule à
  porter le risque réel.
- **AC4** — Le label/trailer est déclaré dans `.github/labels.yml` s'il s'agit
  d'un label (sinon il est supprimé en silence par `delete-other-labels: true`).
  → **DÉJÀ SATISFAIT** : `pipeline-exempt` est à `.github/labels.yml:118` (R2).
  Aucun label neuf n'est introduit, donc aucune ligne à ajouter.

**Dérivées du plan, testables :**

- **AC5** — Un `block[pipeline]` posté via `run_gh pr review` sur une PR dont
  `author.login` ∈ `AUTOMATED_PR_AUTHORS` est **refusé avant le
  sous-processus** ; le corps du refus nomme Step 1.6 et les deux issues
  correctes. (V1, V2)
- **AC6** — Un `pass` posté sur une PR dependabot dont le titre porte un saut de
  majeure et dont le corps ne porte pas `API-SURFACE:` est **refusé**. Un
  `0.x → 0.x` (#2453) et un majeure **avec** `API-SURFACE:` passent. (V1, V2)
- **AC7** — Tout terme illisible (réseau, timeout, titre hors forme, version non
  parsable, token absent) produit une **abstention nommée** et laisse passer le
  `pr review`. (V3)
- **AC8** — `AUTOMATED_PR_AUTHORS` a un seul propriétaire, tenu par un scan de
  parité **bidirectionnel** à allowlist vide. (V4)
- **AC9** — Les deux scans de prompt de V5 portent une assertion d'anti-vacuité
  et rougissent sur un prompt réinjecté (contrôle négatif vu rouge).
- **AC10** — `verify-pipeline.sh`, `verify-pipeline-test.sh` et
  `.github/labels.yml` sont **inchangés**. (V7)

---

## 8. Ce que ce travail n'achète PAS

- **Il ne produit aucune exemption nouvelle.** L'exemption existe ; ce qui change
  est que le verdict qui la contredit devient **impossible à poster**, et non
  seulement déconseillé.
- **Il ne rejoue pas les cinq verdicts mesurés.** Une garde pré-subprocess agit
  sur des appels futurs. Les cinq PR restent à disposer à la main.
- **Il ne garantit pas qu'un bump de majeure soit correctement revu** : il
  garantit qu'un `pass` ne peut pas être posté **sans que la vérification soit
  affirmée**. Affirmer sans vérifier reste possible — c'est la famille
  `assert_grounded` (mika#1331), un autre axe.
- **Il ne couvre pas les bumps groupés** (`Bump the <group> group with N
  updates`) : le titre ne porte aucun couple de versions et B2 s'abstient
  (§ 3.3). Un groupe portant un saut de majeure n'est pas vu. **Suivi nommé**,
  précondition : une mesure montrant qu'un groupe a porté une majeure.
- **Il n'ajoute aucun compteur de « combien de PR dependabot le rail a-t-il
  mergées »** : les seuls instruments sont les sondes du § 6, et **leur silence
  ne prouve rien tant que personne ne les exécute**.

## 9. Hors périmètre, délibérément

- **`verify-pipeline.sh`** — son exemption est correcte et mesurée (R1) ; y
  toucher pour un défaut qui vit ailleurs est exactement l'erreur d'attribution
  que ce plan corrige. Aucune ligne.
- **`.github/labels.yml`** — R2, déjà satisfait.
- **`ci.yml`** — son exclusion par préfixe de branche répond à une autre question
  (*« vaut-il de dépenser un runner ? »*) et `verify-pipeline.sh` explique par
  écrit pourquoi les deux listes ne sont **pas** liées par un lint de parité.
- **`number` absent de `SAFE_FIELDS` de `qa_pr_view.sh`** alors que Step 2B
  l'exige pour son payload synthétique — défaut réel trouvé en chemin, sans
  rapport avec le verdict dependabot (le script lit `.pull_request.user.login`,
  pas `.number`). **Ticket de suivi.**
- **`isDraft` illisible par qa-review** — déjà nommé comme suivi ailleurs dans le
  dépôt, population disjointe.
- **La règle `0.x` de semver** — refusée sur mesure (§ 3.3), à rouvrir avec un
  compte si une rupture `0.x` passe au travers.
- **Le tagging des mémoires défensives de mika-qa** — la cause probable de L3,
  dont mika#2237 a écrit que sa précondition est un **plateau** mesuré plutôt
  qu'une intuition. S1 est la mesure qui l'ouvrirait.
- **Le merge automatique des PR dependabot** — ce plan retire un faux `block` du
  chemin ; il ne décide pas ce que le rail fait d'un `pass`.
