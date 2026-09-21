# mika#2357 — le dormeur des callbacks muets sous glm-5.3 a un site d'inscription, et sa condition de réveil doit être vérifiable

**Ticket :** `mika issue#2357` · **Type :** chore (registre + documentation) · **Date :** 2026-09-21

---

## Ce que le ticket demande

Le corps décrit une classe mesurée les 2026-09-16 et 2026-09-17 : neuf tours
`callback-*` de mika-dev partis en `stop_reason: error`, `input_tokens: 0`,
latence ~420 s ou ~480 s, **tous sous glm-5.3, zéro sous glm-5.2**. Le
contournement retenu par Vincent le 17/09 est le retour de mika-dev à glm-5.2
pour les callbacks (65 tours / 0 erreur / max 108 s). Le commentaire 1 déclare
le ticket **DORMEUR**, porteur de la **racine** — *pourquoi glm-5.3 rend zéro
sur les tours callback* — que le contournement masque sans résoudre, avec deux
conditions de réveil écrites.

Le commentaire 2, du 2026-09-21, pose la question à trancher :

> Promu `ready` par la garde (21/09 02:10 local) : bassin vide, ticket vivant p1
> substrat non parqué, aucune PR ouverte. **Le grooming moteur dira s'il est
> encore d'actualité** depuis le passage des agents sur glm-5.2 via OpenRouter.

---

## Ce que la lecture du dépôt a déplacé

Quatre faits, chacun mesuré dans le dépôt, déplacent le travail. Ils sont le
premier livrable : sans eux, la pente naturelle est de rejouer la comparaison
5.2/5.3 et de reproduire le trou.

### M1 — Le dormeur est déclaré dans un commentaire et **absent du registre**. C'est la cause directe de la promotion parasite.

`docs/dormeurs.md` est le registre ratifié le 2026-09-03 : *« la visibilité
change de support : le registre versionné remplace le ticket ouvert »*. Il
compte dix entrées ; **`grep 2357 docs/dormeurs.md` rend zéro ligne.**

La garde d'alimentation n'a donc rien fait d'anormal : elle a vu un ticket
ouvert, p1, non parqué, dans un bassin vide, et l'a promu. La condition de
réveil écrite dans le corps d'un commentaire n'est lisible par aucun mécanisme
— ni par la garde, ni par `is_feeder_excluded`, ni par un opérateur qui ne
déroule pas les commentaires.

**Conséquence, et c'est le défaut à fermer :** tant que #2357 reste un ticket
ouvert sans ligne au registre, il sera re-promu à chaque bassin vide, consommera
un créneau `groom`, et produira à chaque fois un plan dont la conclusion sera
celle-ci. Le coût n'est pas théorique — ce grooming en est la première
occurrence.

### M2 — La géométrie que le ticket invoque n'est pas celle que le dépôt déclare, et sa provenance n'a jamais été lue.

Le ticket raisonne sur « 2 tentatives × 240 s depuis le flip 240/600 de
mika#2331 » et en tire *« 480 s = 2 × 240 »*. Deux faits contredisent la
prémisse :

- **mika#2331 ne flippe aucune valeur.** Son entrée au `CLAUDE.md` dit
  l'inverse en toutes lettres (« Aucune valeur de réglage n'a bougé »), et c'est
  mika#2342 qui porte cette phrase.
- **`MIKA_DEV_CONFIG` (`crates/mika-agent/src/well_known_agents.rs:181`) ne
  déclare ni `llm_http_timeout_secs` ni `agent_total_timeout_secs`** — donc les
  défauts de flotte, **120/300**. Le figeage mika#2280
  (`mika2280_the_three_shipped_geometries_and_their_verdict`) l'inscrit
  explicitement : mika-dev = `http_timeout_secs: 120, max_tokens: 8_192`.

Si le runtime tourne bien à 240/600, cette géométrie vient d'une **variable
d'environnement de service** qui écrase le per-agent — exactement la cascade que
mika#2293 a rendue lisible, et le cas que mika#2342 a déjà dû nommer pour
mika-arch (« si c'est exact, le 240/900 posé à mika-arch par mika#2189 est
**écrasé** par une variable de service »). Même classe, deuxième agent.

**Ce que cela invalide :** le calcul « 480 = 2 × 240 » repose sur un plafond dont
la provenance n'a pas été établie. Il peut être juste ; il n'est pas *démontré*.
La lecture qui le trancherait existe depuis mika#2293 et n'a pas été faite :

```bash
grep llm_budget_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-dev")
        | {model, model_source, http_timeout_secs, agent_total_timeout_secs,
           http_source, total_source, max_attempts, effective_max_attempts}'
```

### M3 — Les instruments qui trancheraient la racine ont été mergés **le jour même** des mesures. La classe n'a donc jamais été observée avec eux.

| instrument | ce qu'il dit | merge |
|---|---|---|
| mika#2331 — `request_bytes` / `system_prompt_bytes` sur `turn_usage`, **y compris sur le bras `Err`** | la taille du brief d'un tour qui a échoué, là où `input_tokens` vaut 0 | 2026-09-17 |
| mika#2342 — `llm_call_attempt` (avant chaque `send_once`) + `llm_call_watchdog` | combien de tentatives, de quelle durée, et si reqwest a borné | 2026-09-17 |
| mika#2362 — `retrying` honnête, `deadline_abort` distinct | si une tentative a réellement tourné | 2026-09-17 |
| mika#2280 — `cap_exhausted` sur `llm_call_attempt` | si la coupure est **au plafond** (le modèle générait encore) ou n'importe quand | postérieur |

Les neuf tours du ticket sont datés du 16/09 et du 17/09 à 09:41–09:59Z ; le
contournement est posé à **10:05Z**. Aucun de ces instruments ne tournait
alors. **`input_tokens: 0` — la preuve centrale du ticket — est précisément la
ligne que mika#2331 a corrigée** : le bras `Err` écrivait des zéros littéraux et
ne disait rien de la taille du brief qui avait hangué.

**Conséquence :** la classe n'a jamais été mesurée par ce qui la trancherait. Et
depuis le 17/09 le contournement tient, donc **les instruments n'ont pas de
population à mesurer**. C'est la structure de mika#2272 : *zéro était l'absence
de mesure, pas la présence de prudence.*

### M4 — La corrélation structurelle candidate est **fausse sur l'axe skills** pour mika-dev, et l'hypothèse « historique » tombe aussi.

L'entrée `CLAUDE.md` § *Lire un hang LLM « 420 s sans octet »* (mika#2331) nomme
la seule chose qui sépare structurellement la population callback : le tour
callback sélectionne ses skills par `callback_safe_skills()` — `always_on` **plus
les dépendances transitives** — injecté sans plafond global. Mesuré ici pour
mika-dev, depuis les manifestes livrés et l'allowlist de `MIKA_DEV_IDENTITY` :

| skill | rôle | octets |
|---|---|---|
| `self-dev` | seul `always_on` de l'allowlist mika-dev | 62 790 |
| `build-mika`, `deploy-mika`, `dev-pilot`, `dev-groom`, `resolve-pr-conflicts`, `browser-control` | dépendances déclarées de `self-dev` | 13 814 |
| **total** | | **76 604** |

Deux observations en découlent, de sens opposé :

- **Ce n'est pas un écart callback-vs-conversation.** `self-dev` est `always_on`,
  et `match_skills()` résout les dépendances par le même BFS (le doc-comment de
  `callback_safe_skills` le dit : *« same algorithm as `match_skills()` »*). Un
  tour conversationnel de mika-dev porte donc la **même** masse. L'axe skills ne
  peut pas expliquer une défaillance propre aux callbacks.
- **L'axe historique ne le peut pas non plus.** `rebuild_context` n'a **qu'un
  seul site de production**, `run_agent_inner` (`agent_loop/mod.rs:4571`) — le
  chemin conversation. `run_silent_agent` ne reconstruit aucune fenêtre
  conversationnelle. La fuite `HistoryScope::Agent` que mika#2295/#2330 ont
  fermée pour mika-arch **ne s'applique pas** à un tour callback. *(Cette
  hypothèse a été formée puis écartée en cours de grooming ; elle est écrite
  ici pour qu'elle ne soit pas reformée au réveil.)*

Reste donc, pour expliquer les ~47 000 `input_tokens` d'un callback sain :
76,6 KB de skills (~19 k tokens), la core memory (≤ 2 500 tokens), le framing, et
le résultat de callback (capé à 10 240 B par `format_callback_framing`). **Le
compte n'y est pas**, et c'est `request_bytes` / `system_prompt_bytes` qui le
feront — pas un raisonnement.

**Observation annexe, à porter au réveil :** un tour callback de mika-dev
embarque les 62,8 KB de `self-dev` (l'orchestrateur) et **n'embarque pas**
`self-dev-callback` (14 482 B), le skill qui prolonge précisément ce tour —
`callback_safe_skills()` ne suit que les arêtes **sortantes**, et
`self-dev-callback` déclare `dependencies = ["self-dev"]`, pas l'inverse. C'est
l'exception nommée `CALLBACK_REACHABILITY_EXCEPTION` du test
`mika2355_every_bundled_callback_handler_is_reachable_on_a_callback_turn`, dont
le tracker est **mika#2356**. Ce n'est pas la racine de #2357, mais c'est la même
zone, et le remède candidat ci-dessous la traverse.

---

## La réponse à la question posée

**Le ticket est encore dû, et il n'est pas actionnable.** Les deux moitiés
comptent :

- *Encore dû* — la racine n'est pas résolue. Le contournement 5.2 la masque,
  c'est ce que le commentaire 1 dit et rien ne l'a démenti.
- *Pas actionnable* — la population à mesurer n'existe plus par construction
  (glm-5.2 en vigueur), les instruments qui la trancheraient n'ont jamais eu
  cette population, et **aucun remède ne peut être choisi sans cette mesure**.
  Poser un remède ici serait poser un correctif sur une cause non établie.

C'est la définition littérale d'un dormeur au sens de `docs/dormeurs.md` : *« un
travail réellement dû dont la condition d'exécution n'est pas remplie
aujourd'hui »*. Le travail consiste donc à **l'inscrire correctement**, pas à le
résoudre ni à le fermer en silence.

**Ce plan ne livre aucun code.** C'est un résultat, énoncé comme tel : les
quatre mesures ci-dessus établissent qu'il n'y a rien à corriger dans le moteur
aujourd'hui, et que le geste dû est un geste de registre. Un plan qui
inventerait un livrable de code pour ne pas rendre un plan court produirait un
correctif sur une cause non démontrée.

---

## Requirements

- **R1** — Inscrire #2357 au registre `docs/dormeurs.md` avec une condition de
  réveil satisfaisant le contrat du fichier : *« un lecteur peut dire, **sans
  contexte**, si elle est remplie »*.
- **R2** — La condition de réveil du commentaire 1 est à **reformuler**, pas à
  recopier. Sa branche 2 — *« Vincent décide de re-tenter glm-5.3 »* — est du
  type que le contrat refuse explicitement (« si ça revient », « plus tard ») :
  elle nomme une intention, pas un état vérifiable.
- **R3** — Porter au ticket la **procédure de mesure** à suivre au réveil, avec
  les instruments d'aujourd'hui et non ceux du 17/09. Sans cela, le réveil
  rejouera la comparaison 5.2/5.3 et reproduira le trou de M3.
- **R4** — Nommer le **remède candidat** et sa condition, pour que le réveil
  n'ait pas à le redécouvrir — sans le livrer ni le préjuger.
- **R5** — Ne modifier aucune valeur de configuration : ni modèle, ni plafond,
  ni enveloppe, ni `llm_max_tokens`. Le contournement en vigueur est une
  décision opérateur du 17/09 et n'est pas le sujet de ce ticket.
- **R6** — Ne pas ajouter d'instrument. M3 établit que les instruments existent
  et que c'est la population qui manque ; en ajouter un serait construire un
  second silence à côté du premier.

---

## Unités de travail

### U1 — Inscrire #2357 au registre des dormeurs (R1, R2)

Ajouter une ligne au tableau `## Registre` de `docs/dormeurs.md`, dans la forme
des dix existantes :

| ticket | sujet | condition de réveil |
|---|---|---|
| [#2357](https://github.com/senara-solutions/mika/issues/2357) | racine des tours callback muets de mika-dev sous glm-5.3 (`input_tokens: 0`, `stop_reason: error`, muet à 240 s comme à 420 s) | l'une des deux : **(a)** `grep llm_budget_resolved "$MIKA_SPIRIT_LOG_FILE" \| jq -r 'select(.agent_id=="mika-dev") \| .model' \| tail -1` rend autre chose que `glm-5.2` — c'est-à-dire qu'un modèle autre est **effectivement** en service pour mika-dev ; **ou (b)** `grep turn_usage "$MIKA_SPIRIT_LOG_FILE" \| jq 'select((.session_id//"")\|startswith("callback-")) \| select(.model=="glm-5.2" and .status=="error")'` rend **au moins une ligne** — la régression du contournement |

**Pourquoi cette reformulation de (a).** Le commentaire 1 écrit « Vincent décide
de re-tenter glm-5.3 » : une intention, que personne ne peut vérifier depuis le
dépôt. La forme retenue teste l'**état observable** qui en résulte — et elle le
teste sur `llm_budget_resolved`, c'est-à-dire sur le **modèle réellement en
service avec sa provenance** (mika#2328 U2), jamais sur `MIKA_DEV_CONFIG`. Cette
distinction est la leçon de mika#2328, mesurée : *« `zai_model = "glm-5.2"` est
la seule ligne de modèle que ce fichier ait jamais déclarée pour mika-qa […] le
5.3 qui a produit l'incident était une édition **hors dépôt** »*. Le doc-comment
de `MIKA_DEV_CONFIG` porte le même avertissement pour mika-dev, en toutes
lettres : *« this constant's source has DRIFTED from its runtime »*. Une
condition de réveil qui lirait la constante serait donc vérifiable **et fausse**.

**(b) est conservée telle quelle** : elle était déjà vérifiable sans contexte, et
c'est la moitié qui protège contre la régression silencieuse du contournement.

### U2 — Porter au corps du ticket la procédure de mesure du réveil (R3)

Le ticket porte aujourd'hui une « Sonde de reproduction » qui compte des
callbacks par modèle. C'était la bonne sonde le 17/09 ; elle est insuffisante
aujourd'hui, parce qu'elle ne peut dire ni la taille du brief, ni le nombre de
tentatives, ni si la coupure est au plafond. Ajouter au corps une section
**« Au réveil : mesurer avant de remédier »** portant :

1. **Établir la géométrie et sa provenance d'abord** (M2) — `llm_budget_resolved`
   pour mika-dev. `http_source: process_env` signifie qu'une variable de service
   écrase le per-agent, et le remède est de la **retirer**, pas de toucher une
   constante. C'est l'étape 0 de la procédure mika#2331, et elle peut clore le
   sujet sans une ligne de code.
2. **Compter les tentatives** — `llm_call_attempt` avec le garde
   `.event == "llm_call_attempt"` (le garde est nécessaire : sans lui le grep
   capte l'homonyme de mika#2342 et les corps DEBUG).
3. **Mesurer le brief des tours en échec** — `turn_usage` filtré sur
   `.status == "error"`, champs `request_bytes` / `system_prompt_bytes`. **C'est
   l'instrument qui n'existait pas le 17/09**, et c'est lui qui remplace
   `input_tokens: 0`.
4. **Trancher par la table à quatre branches** de `CLAUDE.md` § *Lire un hang LLM
   « 420 s sans octet »*, sans en réécrire une nouvelle.

**Halte explicite à écrire dans le ticket :** si les tours muets suivants ne
portent **aucune** ligne `llm_call_attempt`, la panne est **en amont de l'appel
HTTP** et toute la lignée « retry / borne de prompt » est hors sujet. C'est un
résultat, pas un échec de l'instrument.

### U3 — Nommer le remède candidat et sa condition (R4)

Ajouter au corps du ticket, sous la procédure, le remède candidat **avec sa
condition**, pour qu'il ne soit ni redécouvert ni appliqué prématurément :

> **Remède candidat — borner le brief du tour callback.** Un tour callback de
> mika-dev porte **76 604 octets** de prompt de skills (`self-dev` 62 790 + six
> dépendances), soit l'intégralité du prompt d'orchestration, alors que le
> contrat du tour est porté par `format_callback_framing`. Le mécanisme de
> restriction existe déjà et est **strictement soustractif** :
> `SkillRegistry::apply_only_skills` (mika#2363), livré pour la même raison sur
> mika-arch — où il a retiré 23,5 KB sur 59,8 KB.
>
> **Condition d'application :** la branche 2 de la table mika#2331 — `request_bytes`
> des tours en échec **nettement supérieur** à celui des tours sains du même
> agent. Tant que cette mesure n'est pas faite, ce remède est une hypothèse.
>
> **Deux pièges à connaître avant d'y toucher.** (i) L'axe skills est
> **identique** entre un tour callback et un tour conversationnel de mika-dev
> (`self-dev` est `always_on`) : restreindre le callback ne réduit donc pas un
> écart, il réduit une masse commune — le gain est réel, la causalité ne l'est
> pas. (ii) `self-dev-callback` (14 482 o), le skill qui prolonge ce tour,
> n'est **pas** atteignable aujourd'hui (mika#2356) ; toute restriction du tour
> callback devra composer avec ce ticket, sous peine de figer l'exception.

### U4 — Fermer le ticket au profit du registre (R1)

Le contrat du registre est explicite sur les deux moitiés : *« le registre
versionné **remplace le ticket ouvert** »*, et la clause **Réveil** dit
*« **rouvrir** le ticket GitHub cité (il conserve tout son historique) et retirer
la ligne d'ici »* — un ticket inscrit est donc un ticket fermé.

Fermer #2357 avec un commentaire final renvoyant à la ligne du registre et à la
procédure d'U2.

**Ceci est une décision opérateur, pas un geste de pilote**, et le plan la pose
comme telle : fermer un p1 portant une racine non résolue engage plus qu'une
étape d'implémentation. Deux arguments la soutiennent, et un coût est à peser :

- **Pour** — c'est le geste que le contrat prescrit, et il est la seule chose qui
  ferme la boucle de promotion parasite de M1. Sans lui, U1 à U3 sont écrits et
  le ticket sera re-promu au prochain bassin vide.
- **Pour** — le registre a été ratifié précisément pour que *« le compte d'issues
  cesse de mélanger ce qui reste à faire avec ce qui attend le monde
  extérieur »*. #2357 attend une décision de modèle : c'est la seconde catégorie.
- **Coût nommé** — un p1 fermé sort des tableaux de bord qui comptent les p1
  ouverts. Le registre est versionné et relu, mais il n'est pas un tableau de
  bord. Si l'opérateur juge que cette racine doit rester visible dans le compte
  des p1, **U4 est à ne pas exécuter** et U1–U3 valent seuls — auquel cas il
  faut poser un label de parcage (`operator-review` ou `blocked`), sans quoi M1
  reste ouvert et la promotion se rejouera.

---

## Verification contract

Aucun test automatisé n'est ajouté ni modifié : le livrable est documentaire et
les quatre mesures établissent qu'il n'y a pas de comportement moteur à figer.
La vérification est faite par relecture, sur quatre points vérifiables :

1. **`grep 2357 docs/dormeurs.md`** rend la nouvelle ligne.
2. **Les deux branches de la condition de réveil sont des commandes** rendant un
   résultat interprétable sans contexte — critère du § *Contrat d'une entrée*.
   Ni « plus tard », ni « si ça revient », ni « quand Vincent décidera ».
3. **La condition (a) lit `llm_budget_resolved`, jamais `MIKA_DEV_CONFIG`** —
   sans quoi elle serait vérifiable et fausse (M1 de mika#2328, et le
   doc-comment de `MIKA_DEV_CONFIG` lui-même).
4. **`git diff --stat` ne touche que `docs/`.** Aucun fichier sous `crates/`,
   `skills/` ou `.github/` n'est modifié — c'est la traduction mécanique de R5
   et R6.

`make lint` / `make test` restent verts par construction (aucun fichier Rust
touché) ; ils seront exécutés par la CI de la PR.

---

## Definition of Done

- [ ] `docs/dormeurs.md` porte une ligne #2357 conforme au contrat du fichier.
- [ ] Le corps du ticket #2357 porte la section « Au réveil : mesurer avant de
      remédier » (U2) et le remède candidat avec sa condition (U3).
- [ ] La rectification M2 (la géométrie 240/600 n'est pas déclarée par le dépôt)
      est écrite au ticket, de sorte que le réveil ne reparte pas du calcul
      « 480 = 2 × 240 » comme d'un acquis.
- [ ] La rectification M4 (l'axe skills est identique callback/conversation ;
      l'axe historique ne s'applique pas au tour silencieux) est écrite, de
      sorte que ces deux hypothèses ne soient pas reformées.
- [ ] Le diff ne touche que `docs/`.
- [ ] La décision d'U4 (fermeture au profit du registre, ou parcage par label)
      est prise par l'opérateur et exécutée.

---

## Acceptance criteria

*Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés des Requirements et du Verification contract.*

- **AC1** — `grep -c 2357 docs/dormeurs.md` rend au moins 1, et la ligne est
  dans le tableau `## Registre`, à la forme des dix entrées existantes
  (ticket lié, sujet, condition de réveil).
- **AC2** — Les deux branches de la condition de réveil sont exécutables telles
  quelles et rendent un résultat qu'un lecteur interprète sans contexte : (a)
  une valeur de modèle, (b) un compte de lignes. Aucune ne contient de formule
  d'intention.
- **AC3** — La branche (a) interroge `llm_budget_resolved` et non
  `MIKA_DEV_CONFIG` ni `well_known_agents.rs`.
- **AC4** — Le corps de #2357 porte les quatre étapes de mesure d'U2, dont
  l'étape 0 (géométrie et **provenance** via `llm_budget_resolved`) en premier,
  et la halte « aucune ligne `llm_call_attempt` ⇒ la panne est en amont de
  l'appel HTTP, la lignée retry/borne est hors sujet ».
- **AC5** — Le corps de #2357 porte le remède candidat, sa condition
  d'application (branche 2 de la table mika#2331) et les deux pièges d'U3.
- **AC6** — Aucun fichier hors `docs/` n'est modifié. En particulier :
  `MIKA_DEV_CONFIG` est inchangé (modèle, `llm_max_tokens`, absence de plafond
  et d'enveloppe), et `mika2280_the_three_shipped_geometries_and_their_verdict`
  n'est pas touché.
- **AC7** — Aucun événement de journal, compteur ou ligne `audit_events`
  nouveau n'est introduit (R6).

---

## Hors périmètre, délibérément

- **Le retour de mika-dev à glm-5.3.** Décision opérateur, et c'est la branche
  (a) de la condition de réveil — pas un livrable.
- **La cause côté fournisseur** (pourquoi glm-5.3 rend un corps vide sur une
  fraction des requêtes). Non atteignable depuis ce dépôt ; ce travail rend la
  classe mesurable au réveil, il ne la fait pas disparaître.
- **Borner le brief du tour callback.** Nommé en U3 avec sa condition ;
  conditionné à une mesure qui ne peut pas être faite aujourd'hui.
- **mika#2356** (`self-dev-callback` inatteignable sur un tour de callback).
  Défaut réel, même zone, ticket distinct qui porte déjà son tracker et son
  test auto-nettoyant.
- **La divergence constante/runtime de `MIKA_DEV_CONFIG`** (le doc-comment dit
  que la constante déclare 5.2 pendant que les plans de mika#2179 et mika#2189
  mesuraient mika-dev sur 5.3). Réconcilier cette dérive exige une calibration
  passante sur le modèle réellement en service (mika#1190) ; le doc-comment la
  désigne déjà comme « its own piece of work ». **Ticket de suivi** — et noter
  que la branche (a) d'U1 mesure cette dérive au passage.
- **Le bornage de la fenêtre d'historique de mika-dev** (`[context.history]`
  absent, donc `HistoryScope::Agent` sans `max_tokens`). Réel, et hors sujet
  ici : M4 établit que le chemin silencieux ne reconstruit aucune fenêtre
  conversationnelle, donc ce défaut ne touche pas les tours callback. Il touche
  les tours **conversationnels** de mika-dev — **ticket de suivi**, dont le
  préalable est une mesure `context_window_assembled` sur cet agent.

---

## Risques et haltes

- **Halte 1 — un tour callback repart en erreur sous glm-5.2 avant que ce plan
  ne soit livré.** La branche (b) est alors déjà remplie : le ticket n'est pas
  un dormeur, il est **actif**. Ne pas l'inscrire au registre ; suivre la
  procédure d'U2 sur la population qui vient d'apparaître.
- **Halte 2 — la mesure de géométrie rend `http_source: agent_config` à 240.**
  Alors M2 est faux, le dépôt a bougé depuis ce grooming, et l'arithmétique du
  ticket doit être refaite avant toute conclusion. Ne pas livrer U1 sans
  corriger M2.
- **Halte 3 — `llm_budget_resolved` ne rend aucune ligne pour mika-dev.** Le
  binaire déployé est antérieur à mika#2293 : c'est **le déploiement** qu'il
  faut établir avant de conclure quoi que ce soit sur la configuration (classe
  mika#2340). Ne jamais lire ce silence comme « la configuration est celle du
  dépôt ».
- **Risque assumé — la condition de réveil (a) est vérifiable mais passive.**
  Personne n'exécute cette commande chaque jour. C'est la limite propre à tout
  le registre, écrite dans son contrat, et elle est acceptée : le registre est
  relu, pas surveillé. Le réveil réel viendra de la décision opérateur qui
  change le modèle — la commande sert à ce que ce réveil soit **vérifiable**,
  pas à le déclencher.
