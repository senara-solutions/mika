# mika#2357 — le dormeur est inscrit, et sa condition de réveil ne peut pas détecter son propre réveil

**Ticket :** `mika issue#2357` · **Type :** chore (registre) · **Plan initial :** 2026-09-21 · **Révisé :** 2026-09-24

---

## Ce que le ticket demande

Le commentaire 2, du 2026-09-21, pose la question à trancher :

> Promu `ready` par la garde (21/09 02:10 local) : bassin vide, ticket vivant p1
> substrat non parqué, aucune PR ouverte. **Le grooming moteur dira s'il est
> encore d'actualité** depuis le passage des agents sur glm-5.2 via OpenRouter.

Le corps décrit une classe mesurée les 16 et 17/09 : neuf tours `callback-*` de
mika-dev partis en `stop_reason: error`, `input_tokens: 0`, latence ~420 s ou
~480 s, **tous sous glm-5.3, zéro sous glm-5.2**. Le contournement retenu par
Vincent le 17/09 est le retour de mika-dev à glm-5.2 (65 tours / 0 erreur /
max 108 s). Le commentaire 1 déclare le ticket **DORMEUR**, porteur de la
**racine** que le contournement masque sans résoudre.

---

## Ce que la lecture du dépôt a déplacé depuis le plan du 21/09

**Ce plan a déjà été écrit une fois, et il a été partiellement consommé
trois heures plus tard.** Les cinq mesures ci-dessous sont le premier livrable :
sans elles, la pente est de rejouer le plan du 21/09, dont la mesure centrale
est aujourd'hui fausse.

### M1 — Le dormeur EST inscrit. U1 du plan initial est consommé, par une autre main.

Le plan du 21/09 posait, comme cause directe de la promotion parasite :
*« `grep 2357 docs/dormeurs.md` rend zéro ligne »*. C'était vrai à 02:08:24.

`acd7ac2f` (« docs(dormeurs): inscrire #2357 (callback muets glm-5.3) au
registre », PR #2447) a inscrit la ligne à **05:00:17 le 21/09** — environ trois
heures après le commit du plan. `grep -c 2357 docs/dormeurs.md` rend **1**.

Le corps du ticket porte désormais l'encart *« Dormeur visible (inscrit
2026-09-21) … Inscrit aussi au registre `docs/dormeurs.md` »*.

**Ce qui reste dû n'est donc plus de créer la ligne, mais de la corriger** — et
M2 établit que la correction est réelle, pas cosmétique.

### M2 — La condition de réveil inscrite est structurellement incapable de détecter le réveil qu'elle décrit.

La ligne 48 de `docs/dormeurs.md` porte :

> quand le modèle actif de mika-dev redevient `glm-5.3` (essai relancé) —
> **vérifiable via `config.toml`/`turn_usage`** ; aujourd'hui `glm-5.2`
> (possédé, quality-first), donc dormant

`config.toml` est précisément la lecture que ce dépôt établit comme trompeuse,
et il le fait **dans le doc-comment de la constante concernée**
(`crates/mika-agent/src/well_known_agents.rs:165-192`) :

> *this constant's source has DRIFTED from its runtime. It declares
> `z-ai/glm-5.2` while the plans of mika#2179 and mika#2189 measure mika-dev on
> `z-ai/glm-5.3`*

Et `MIKA_DEV_CONFIG` déclare `openrouter_model = "z-ai/glm-5.2"` **depuis le
2026-06-29** (`25a8ef4b`, mika#1633) — c'est-à-dire **pendant toute la fenêtre
où le défaut a été mesuré**. La leçon est celle de mika#2328, mesurée sur
mika-qa : *« le 5.3 qui a produit l'incident était une édition hors dépôt »*.

**Conséquence, et c'est le défaut à fermer :** un opérateur qui exécute la
condition de réveil telle qu'écrite lit `glm-5.2`, conclut « dormant », et
conclura **exactement la même chose le jour où mika-dev tournera glm-5.3**. La
condition ne peut pas être remplie. *Une condition de réveil fausse est pire
qu'absente : elle sera exécutée, elle rendra une réponse, et la réponse sera
fausse dans le sens rassurant.*

Deux défauts mineurs s'y ajoutent. *« (essai relancé) »* nomme une **intention**,
que le contrat du fichier refuse explicitement (« plus tard », « si ça revient » :
non). Et la **branche (b) du commentaire 1 est absente** du registre — la
régression du contournement (un callback en erreur sous glm-5.2) n'y figure nulle
part, alors que c'est la moitié qui protège contre un réveil silencieux.

### M3 — Le remède que le plan du 21/09 nommait en « ticket de suivi » est livré.

Le plan initial renvoyait la dérive constante↔runtime à un suivi, et posait en
risque assumé : *« la condition de réveil (a) est vérifiable mais passive.
Personne n'exécute cette commande chaque jour. »* **Les deux sont périmés.**

| livré depuis | ce que ça donne |
|---|---|
| **mika#2457** (`mika agents budget --agent mika-dev`) | le couple `(provider, modèle)` que l'agent fait **tourner**, attesté par mika-spirit, avec sa provenance et son `resolved_at`. Serveur injoignable ou 404 ⇒ *« non attesté »*, **aucune valeur locale affichée** |
| **mika#2473** (`well_known_model_drift`) | une ligne WARN **par agent et par `init_agent`** quand le modèle résolu diffère du déclaré — donc la dérive est **poussée**, elle ne s'interroge plus |

Le doc-comment de `MIKA_DEV_CONFIG` pointe d'ailleurs déjà la commande, dans un
bloc dédié. **La condition de réveil (a) cesse d'être passive** : un retour de
mika-dev à glm-5.3 hors dépôt s'annonce désormais au démarrage suivant.

### M4 — La procédure de mesure du réveil existe déjà, ailleurs, et n'a pas à être réécrite.

U2 du plan initial voulait porter au corps du ticket une procédure « mesurer
avant de remédier ». Elle est **déjà écrite**, dans le `CLAUDE.md` racine,
§ *Lire un hang LLM « 420 s sans octet »* (mika#2331) : établir la géométrie et
sa provenance d'abord, compter les tentatives avec le garde `.event ==
"llm_call_attempt"`, mesurer `request_bytes` / `system_prompt_bytes` sur les
tours en échec, puis trancher par une table à quatre branches — critère de halte
compris.

Ce qui reste utile est un **pointeur**, parce que le point que le réveil doit
savoir est contre-intuitif : les instruments qui trancheraient cette classe
(mika#2331, #2342, #2362) ont été **mergés le 17/09**, c'est-à-dire le jour même
des mesures et avant le contournement de 10:05Z. `input_tokens: 0` — la preuve
centrale du ticket — **est la ligne que mika#2331 a corrigée**. La classe n'a
donc jamais été observée par ce qui la trancherait, et un réveil qui rejouerait
la comparaison 5.2/5.3 reproduirait le trou.

### M5 — Trois des quatre unités du plan initial n'étaient pas livrables par une PR.

U2 et U3 prescrivaient d'éditer le **corps du ticket** ; U4 de le **fermer**. Un
pilote d'implémentation travaille dans un worktree et livre une PR : `gh issue
edit` n'est pas versionné, n'est pas revu, et n'est pas ce qu'une PR contient.
U4 est par ailleurs **déjà tranché dans l'autre sens** — le ticket est resté
ouvert *et* inscrit, ce qui est la disposition que l'opérateur a retenue le
21/09 en posant l'encart dormeur dans le corps.

Ce plan ne livre donc que ce qu'une PR peut livrer : **un fichier versionné.**

---

## La réponse à la question posée

**Le ticket est encore dû, il n'est pas actionnable, et il est désormais
correctement classé — mais sa condition de réveil est fausse, et c'est le
travail.**

- *Encore dû* — la racine n'est pas résolue. Le contournement glm-5.2 la masque ;
  rien depuis le 17/09 ne l'a démentie.
- *Pas actionnable* — la population à mesurer n'existe plus par construction
  (glm-5.2 en vigueur, et le `CLAUDE.md` racine enregistre mika-dev **en phase**
  au 22/09). Aucun remède ne peut être choisi sans cette mesure ; en poser un
  serait corriger une cause non établie.
- *Le résidu réel est petit, et il est réel* — M2. Une condition de réveil qui
  lit `config.toml` répondra « dormant » y compris au réveil.

**Ce plan ne livre aucun code, et c'est un résultat, pas une facilité.** Les cinq
mesures établissent qu'il n'y a rien à corriger dans le moteur aujourd'hui.
Inventer un livrable de code pour ne pas rendre un plan court produirait un
correctif sur une cause non démontrée.

---

## Requirements

- **R1** — Rectifier la condition de réveil de #2357 dans `docs/dormeurs.md` pour
  qu'elle interroge le modèle **en service**, jamais celui que le dépôt déclare.
- **R2** — Y porter les **deux** branches du commentaire 1 : le retour à glm-5.3
  (a) *et* la régression du contournement (b), aujourd'hui absente du registre.
- **R3** — Remplacer la formulation d'intention (« essai relancé ») par un état
  vérifiable, per le contrat du fichier : *« un lecteur peut dire, **sans
  contexte**, si elle est remplie »*.
- **R4** — Porter un pointeur vers la procédure de mesure du réveil (M4), pour
  que le réveil n'applique pas les instruments du 17/09.
- **R5** — Ne modifier aucune valeur de configuration : ni modèle, ni provider,
  ni plafond, ni enveloppe, ni `llm_max_tokens`. Le contournement est une
  décision opérateur du 17/09 et n'est pas le sujet.
- **R6** — N'ajouter aucun instrument. M3 établit qu'ils existent ; en ajouter un
  serait construire un second silence à côté du premier.
- **R7** — Ne toucher qu'à `docs/`.

---

## Unités de travail

### U1 — Rectifier la ligne #2357 du registre (R1–R4)

Remplacer la ligne 48 de `docs/dormeurs.md`. Forme cible (le tableau existant a
trois colonnes : ticket, sujet, condition de réveil) :

- **Colonne « sujet »** — la racine, plus la clause de M4 :
  racine des tours callback muets de mika-dev sous glm-5.3 (`stop_reason: error`,
  `input_tokens: 0`, muet à 240 s comme à 420 s) ; le contournement glm-5.2 la
  masque. Au réveil, mesurer avec les instruments d'aujourd'hui — `CLAUDE.md`
  § *Lire un hang LLM « 420 s sans octet »* — et **non** avec `input_tokens: 0`,
  qui est la ligne que mika#2331 a corrigée le jour même des mesures.

- **Colonne « condition de réveil »** — l'une des deux :
  - **(a)** `mika agents budget --agent mika-dev` rend un `model` qui n'est pas
    un `glm-5.2` — **quel que soit le préfixe de rail** (`z-ai/`, `zai/`,
    `openrouter/z-ai/` : mika#2328 a mesuré que le rail bouge sans que le modèle
    change). C'est le modèle que mika-dev fait **tourner**, attesté par
    mika-spirit (mika#2457), jamais celui que le dépôt déclare. Le même fait est
    **poussé** au démarrage par `well_known_model_drift` (mika#2473).
  - **(b)** `grep turn_usage "$MIKA_SPIRIT_LOG_FILE" | jq 'select((.session_id//"")|startswith("callback-")) | select(.model=="glm-5.2" and .status=="error")'`
    rend **au moins une ligne** — la régression du contournement.

**Pourquoi (a) est reformulée ainsi.** Le commentaire 1 écrit « Vincent décide de
re-tenter glm-5.3 » : une intention que personne ne peut vérifier. La forme
retenue teste l'**état observable** qui en résulte, et elle le teste sur la
surface que mika#2457 a créée pour cette question exacte — le record figé à
l'`init_agent`, jamais une résolution locale, qui lirait le `process_env` du
process lecteur et « affirmerait avec autorité un réglage qui n'est pas en
vigueur ». Une condition qui lirait `config.toml` serait vérifiable **et fausse**
(M2).

**Pourquoi (b) est ajoutée telle quelle.** Elle était déjà vérifiable sans
contexte dans le commentaire 1, et c'est la seule moitié qui protège contre une
régression silencieuse du contournement. Son absence du registre est un trou,
pas un choix : le registre remplace le ticket ouvert comme support de
visibilité, donc une condition qui ne vit que dans un commentaire n'est lisible
par personne.

**Échappement :** les `|` des deux commandes doivent être échappés `\|` dans la
cellule markdown, comme le fait déjà la cellule de #2119.

---

## Verification contract

Aucun test n'est ajouté ni modifié : le livrable est une ligne de documentation,
et les cinq mesures établissent qu'aucun comportement moteur n'est à figer. La
vérification est une relecture, sur cinq points vérifiables :

1. `grep -c 2357 docs/dormeurs.md` rend **1** — une ligne, pas deux (la ligne est
   rectifiée, pas ajoutée à côté de l'ancienne).
2. La ligne ne contient plus la chaîne `config.toml`.
3. La ligne contient `mika agents budget` et `turn_usage`.
4. La ligne ne contient aucune formule d'intention (« essai relancé », « plus
   tard », « si ça revient »).
5. `git diff --stat` ne touche que `docs/dormeurs.md` — traduction mécanique de
   R5, R6 et R7.

`make lint` / `make test` restent verts par construction (aucun fichier Rust
touché) ; la CI de la PR les exécute. Le `canonical-tokens-lint` (mika#2201)
s'applique au fichier modifié : la ligne n'écrit aucun label en instruction et ne
pose aucune clé de callout, donc elle est hors de ses cinq règles.

---

## Definition of Done

- [ ] La ligne #2357 de `docs/dormeurs.md` porte les deux branches (a) et (b),
      chacune exécutable telle quelle.
- [ ] La branche (a) interroge `mika agents budget`, et `config.toml` a disparu
      de la ligne.
- [ ] La colonne « sujet » porte le pointeur vers la procédure de mesure et la
      raison de ne pas repartir de `input_tokens: 0`.
- [ ] Le registre compte toujours le même nombre d'entrées (rectification, pas
      ajout).
- [ ] Le diff ne touche que `docs/dormeurs.md`.

---

## Acceptance criteria

*Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés des Requirements et du Verification contract.*

- **AC1** — `docs/dormeurs.md` contient exactement une ligne #2357, dans le
  tableau `## Registre`, à la forme des onze entrées existantes.
- **AC2** — Les deux branches sont exécutables telles quelles et rendent un
  résultat qu'un lecteur interprète sans contexte : (a) une valeur de modèle,
  (b) un compte de lignes. Aucune ne contient de formule d'intention.
- **AC3** — La branche (a) interroge le modèle **en service** via
  `mika agents budget` ; ni `config.toml`, ni `well_known_agents.rs`, ni la
  constante `MIKA_DEV_CONFIG` n'y sont prescrits comme source.
- **AC4** — La branche (a) est robuste au préfixe de rail : elle ne repose pas
  sur une égalité stricte avec `z-ai/glm-5.2`.
- **AC5** — La branche (b) — la régression du contournement — figure au
  registre, ce qui n'était pas le cas avant ce travail.
- **AC6** — La colonne « sujet » nomme la procédure de mesure à suivre au réveil
  et dit pourquoi `input_tokens: 0` n'est plus la bonne preuve.
- **AC7** — Aucun fichier hors `docs/` n'est modifié. En particulier
  `MIKA_DEV_CONFIG` est inchangé (provider, modèle, `llm_max_tokens`, absence de
  plafond et d'enveloppe).
- **AC8** — Aucun événement de journal, compteur ou ligne `audit_events` nouveau
  n'est introduit (R6).

---

## Hors périmètre, délibérément

- **Tout détecteur.** Ce plan n'en livre aucun — pas de test, pas d'assertion,
  pas de règle de lint, pas de garde CI, pas de scan structurel, pas de garde
  EndTurn — donc aucun chemin de succès du type « aucune violation trouvée »
  n'est créé, et la section `## Fire-Disposition` est sans objet (gate N/A).
  Un détecteur a bien été envisagé puis **écarté** : un test qui vérifierait que
  chaque entrée du registre porte une condition exécutable déborderait le ticket
  (onze entrées, dont plusieurs légitimement sans commande — #2139 attend un
  alignement de versions amont, #1812 une décision opérateur, #1913 une date),
  et « condition exécutable » n'est pas mécaniquement décidable. Le livrer
  serait inventer un livrable.

- **Le retour de mika-dev à glm-5.3.** Décision opérateur, et c'est la branche
  (a) de la condition de réveil — pas un livrable.
- **La cause côté fournisseur** (pourquoi glm-5.3 rend un corps vide sur une
  fraction des requêtes). Non atteignable depuis ce dépôt.
- **Borner le brief du tour callback** via `apply_only_skills` (mika#2363), le
  remède candidat que le plan initial nommait. Il reste conditionné à une mesure
  impossible aujourd'hui, **et le plan initial a lui-même établi qu'il
  n'expliquerait pas la classe** : `self-dev` est `always_on`, donc un tour
  conversationnel de mika-dev porte la même masse de skills qu'un tour callback,
  et l'axe skills ne peut pas expliquer une défaillance propre aux callbacks. La
  note est conservée ici pour qu'elle ne soit pas reformée au réveil, pas comme
  une piste.
- **L'axe historique.** `rebuild_context` n'a qu'un site de production,
  `run_agent_inner` — le chemin conversation ; `run_silent_agent` ne reconstruit
  aucune fenêtre conversationnelle. La fuite `HistoryScope::Agent` fermée par
  mika#2295/#2330 **ne touche pas** les tours callback. Écrit ici pour la même
  raison.
- **mika#2356** (`self-dev-callback` inatteignable sur un tour de callback).
  Défaut réel, même zone, ticket distinct portant déjà son tracker et son test
  auto-nettoyant.
- **La réconciliation de la dérive `MIKA_DEV_CONFIG`.** Elle exige une
  calibration passante sur le modèle réellement en service (mika#1190) ; son
  doc-comment la désigne déjà comme « its own piece of work ». **Ticket de
  suivi** — et noter que la branche (a) la mesure au passage.
- **Fermer #2357 au profit du registre.** Déjà tranché dans l'autre sens le
  21/09 (ouvert *et* inscrit). Décision opérateur, pas un livrable de PR (M5).

---

## Risques et haltes

- **Halte 1 — un tour callback repart en erreur sous glm-5.2 avant la
  livraison.** La branche (b) est alors déjà remplie : le ticket n'est pas un
  dormeur, il est **actif**. Ne pas livrer la rectification comme si de rien
  n'était ; suivre la procédure de M4 sur la population qui vient d'apparaître.
- **Halte 2 — `mika agents budget --agent mika-dev` rend *« non attesté »*.** Ce
  n'est pas un réveil et ce n'est pas un échec de la condition : c'est un serveur
  injoignable, un 404, ou un binaire antérieur à mika#2457 (classe mika#2340).
  Établir le déploiement **avant** toute conclusion sur le modèle. La commande
  est conçue pour n'afficher **aucune** valeur locale dans ce cas, précisément
  pour que ce silence ne se lise pas comme une réponse.
- **Halte 3 — la ligne rend un `model` autre que `glm-5.2` dès la première
  exécution.** Le réveil est **déjà** rempli : mika-dev tourne autre chose que ce
  que le corps du ticket suppose. Ne pas livrer la ligne en l'état comme
  « dormant » — c'est un résultat à porter au ticket, et le dormeur se réveille.
  Le `CLAUDE.md` racine enregistre mika-dev **en phase** au 2026-09-22, donc
  l'attendu est `glm-5.2` ; un écart est une mesure, pas une panne.
- **Risque assumé, réduit mais non nul.** La condition (a) reste une commande que
  personne n'exécute chaque jour — limite propre à tout le registre, écrite dans
  son contrat. Elle est désormais **doublée** par un signal poussé
  (`well_known_model_drift` au démarrage, mika#2473), ce qui n'était pas le cas
  le 21/09 ; mais ce signal ne nomme pas #2357, et rien ne relie automatiquement
  l'un à l'autre. Le réveil réel viendra de la décision opérateur qui change le
  modèle ; la commande sert à ce que ce réveil soit **vérifiable**, pas à le
  déclencher.
