# fix(gateway) — `INTERNAL_REPOS` porte `senara-solutions/claude-pilot`, et l'ancien nom n'y revient pas

**Ticket :** senara-solutions/mika#2666 (`bug`, `p3-nice-to-have`, `ready`, `dispatch:loop`)
**Classe :** hygiène d'une constante indexée par nom de dépôt, après renommage amont.

---

## Contexte

`crates/mika-gateway/src/github.rs:265` déclare `"senara-solutions/claude-pilot-py"`
dans `INTERNAL_REPOS`. Le dépôt a été renommé `senara-solutions/claude-pilot` ;
GitHub met le **nom courant** dans `repository.full_name`, donc cette entrée ne
correspond plus jamais.

**La prémisse du corps du ticket est fausse, et le commentaire de grooming du
2026-10-07 l'a mesurée.** Le corps conclut « les événements de claude-pilot ne
sont plus routés ». Ils le sont. `is_internal_repo` n'est consulté que dans la
branche `Ok(None)` de la résolution de route (`github.rs:1186-1240`), et les deux
branches rendent **la même route** quand `agent_base_url` est posé :

| état du dépôt | route rendue | niveau du log |
|---|---|---|
| dans l'allowlist | `agent_base_url` | `info!` « internal repo resolved via allowlist » |
| hors allowlist, `agent_base_url` posé | `agent_base_url` *(fallback mono-locataire)* | `debug!` « falling back to agent_base_url » |
| hors allowlist, pas d'`agent_base_url` | **aucune — événement abandonné** | `warn!` |

Sur le moteur local (mono-locataire), l'entrée périmée ne change donc **que le
niveau du log**. La disparition de la ligne INFO depuis le 2026-07-07 est cette
différence de niveau, pas une perte. Preuve indépendante que les événements
arrivent : le 2026-10-02, mika-qa a revu claude-pilot#281 dans un tour du canal
`github`, et 590 sessions de ce canal mentionnent claude-pilot depuis le
2026-09-01.

**Ce que le ticket vaut donc, exactement :**

1. Le seul régime où l'entrée périmée **perd** des événements est le troisième
   ligne du tableau — multi-locataire sans `agent_base_url`. Le commentaire du
   code dit déjà qu'en K8s ces dépôts doivent porter une ligne `github_repos`
   explicite, donc cette population est vide aujourd'hui. Le correctif est une
   **assurance** sur ce régime, pas une réparation d'un défaut observé.
2. Un nom mort dans une liste de sécurité, et un test qui l'épingle
   (`github.rs:2058`) — c'est-à-dire un test qui atteste aujourd'hui une entrée
   que rien ne peut plus atteindre. C'est le défaut réel, et il est d'hygiène.
3. **La prémisse R8 de mika#2617 phase C (#2665) ne dépend pas de ce vol.** Le
   corps du ticket l'affirmait ; c'est corrigé ci-dessus. La garde MPC tient sur
   ses trois sites par le type (`MergeClearance`), quel que soit le routage.

Le plan ne maquille pas ce redimensionnement : il le déclare, et il refuse d'en
tirer un livrable plus gros que le constat.

---

## Décisions de grooming

### D1 — L'alias `claude-pilot-py` est RETIRÉ, et la raison est écrite au site

Le ticket laisse la question ouverte (« à trancher et à écrire dans la PR »). La
doctrine existe déjà dans l'arbre,
`docs/solutions/security-issues/repo-name-keyed-guard-goes-inert-on-rename-keep-the-alias-2026-10-05.md`,
et elle énonce la règle **avec son inversion** :

> Pour une liste qui **restreint** (population d'une garde), garder un alias ne
> coûte rien. Pour une liste qui **autorise**, la règle s'inverse : un alias
> élargit l'autorisation, et il faut le peser.

`MPC_GATE_REQUIRED_REPOS` (`mika-common/src/forge_identity.rs:316`) **restreint** —
elle exige une trace de gate avant un merge. Elle garde les deux noms, parce que
GitHub redirige un dépôt renommé : un appelant écrivant encore l'ancien nom
atteint la même PR, et doit trouver la porte **fermée**. C'est correct et ce vol
n'y touche pas.

`INTERNAL_REPOS` **autorise** — elle ouvre une route sans ligne `github_repos`.
Pesons l'alias, comme la doctrine l'exige :

- **Bénéfice : nul.** GitHub n'envoie que le nom courant dans
  `repository.full_name` ; l'entrée alias ne serait jamais évaluée vraie. Il n'y
  a pas d'appelant « qui écrit l'ancien nom » du côté d'un webhook : l'appelant,
  c'est GitHub.
- **Coût : non nul.** Une entrée morte dans une liste de sécurité est
  exactement le défaut que ce ticket corrige. La reconduire sous le nom
  « alias » la rend indéfiniment indiscernable d'une entrée vivante, dans la
  liste même dont le ticket vient de montrer que personne ne relit les entrées.

Donc : **nom courant seul.** Et la décision va dans le doc-comment de la
constante, pas seulement dans le corps de la PR — sans ça, un lecteur futur
compare les deux listes, voit que l'une garde l'alias et l'autre pas, et lit une
incohérence là où il y a deux natures de liste.

### D2 — `forge_identity.rs` garde son alias, et sa justification est AMENDÉE

Le doc-comment de `MPC_GATE_REQUIRED_REPOS` justifie son alias en nommant ce
vol-ci comme appelant périmé :

> Un appelant qui écrit encore l'ancien nom — un prompt périmé, `INTERNAL_REPOS`
> du gateway qui le liste toujours — ne doit pas trouver la porte ouverte pour
> autant.

Après ce vol, cette parenthèse est **fausse**. L'alias MPC reste justifié — la
redirection GitHub vaut pour un `gh pr merge` à la main et pour tout prompt
périmé — mais la justification ne peut plus citer un site qui a migré. Un
doc-comment qui s'appuie sur un fait révolu est la prochaine décision que
quelqu'un défera par erreur. On amende la parenthèse ; **on ne retire pas
l'alias.**

### D3 — Le prompt `qa-review-webhook-success` est DANS le vol, et c'est le livrable le plus conséquent après la constante

`skills/bundled/qa-review-webhook-success/system_prompt.md:21` énumère les
« reviewable repos » de mika-qa et y écrit `claude-pilot-py`. Contrairement à la
constante — inerte en mono-locataire — ce prompt a une conséquence **active** :
mika-qa lit la liste, voit `claude-pilot-py`, et un événement portant
`senara-solutions/claude-pilot` peut lui sembler hors de son périmètre. Le point
2 de son chemin de décision est « Skip if not-our-scope », dont le chemin le plus
probable est un `stop the turn` silencieux.

C'est la raison pour laquelle le vol ne se borne pas au gateway : le seul endroit
où le nom périmé **décide** quelque chose aujourd'hui est un prompt, pas la
constante du titre.

**Conséquence opérationnelle à écrire dans la PR (mika#2340) :** rien ne lit
`skills/bundled/` à l'exécution. Ce changement n'atteint mika-qa qu'après
`make deploy` → seed → lecture. Le merge ne le met pas en service.

### D4 — Le fixture `NOT_DISPATCHABLE` migre, et le test reste vert

`ci_failure_handler.rs:1386` pose `NOT_DISPATCHABLE = "senara-solutions/claude-pilot-py"`.
`DISPATCHABLE_REPOS` contient `mika`, `mika-cloud`, `mika-skills`,
`mika-platform` — **les deux noms** de claude-pilot en sont absents, donc le
fixture reste non-dispatchable sous le nom courant et l'assertion ne bouge pas.
Le migrer coûte une ligne et rend le fixture honnête sur la population qu'il
prétend représenter (un dépôt que GitHub peut réellement nommer). Les deux
doc-comments voisins qui écrivent « it carries `claude-pilot-py` and `wizzard` »
(l. 591-592) et son miroir dans `docs/agent-internals/tools.md` suivent, pour la
même raison.

### D5 — Hors périmètre, nommé plutôt que corrigé en passant

- **`is_internal_repo` reste sensible à la casse et sans `trim`**, là où
  `mpc_gate_required` fait les deux. L'asymétrie est **correcte dans ce
  sens-là** : pour une liste qui autorise, une casse non reconnue est
  *fail-closed* — le dépôt tombe dans le fallback, qui en mono-locataire rend la
  même route et en multi-locataire abandonne l'événement, c'est-à-dire refuse.
  Normaliser serait élargir une autorisation sans qu'aucune mesure l'ait
  demandé. Si un jour une mesure la demande, c'est un ticket.
- **`claude-pilot` reste hors de `DISPATCHABLE_REPOS`** (spawn-only, décision
  opérateur du 2026-08-29). Ce vol rétablit un nom dans la liste de **routage**,
  jamais dans celle de dispatch. Les deux listes répondent à deux questions, et
  `docs/architecture/mika-dev-work-assignment.md` § 2 est le site qui le dit.
- **Les occurrences `claude-pilot-py` qui désignent un CHEMIN LOCAL ne bougent
  pas.** Le répertoire du workspace est `/data/workspace/mika-platform/claude-pilot`,
  mais `docs/solutions/**` compte ~20 occurrences décrivant des faits datés
  (`claude-pilot-py/src/claude_pilot/tier1.py`, `claude-pilot-py#16`, …) et le
  paquet Python s'appelle toujours `claude_pilot`. Réécrire l'historique d'un
  doc de solution est une falsification de sa date, pas une correction. Seules
  les occurrences qui **décrivent l'état courant d'une liste** sont touchées.

---

## Conception

### L1 — La constante et son doc-comment (`crates/mika-gateway/src/github.rs`)

L'entrée devient `"senara-solutions/claude-pilot"`. Le doc-comment existant
(l. 249-260) gagne un paragraphe nommé qui porte D1 : la nature *autorisante* de
la liste, l'inversion de la règle d'alias, et le renvoi au doc de solution et à
`MPC_GATE_REQUIRED_REPOS` pour qu'un lecteur comparant les deux listes trouve la
raison de leur divergence **au site**, sans avoir à retrouver ce ticket.

### L2 — Les tests (`crates/mika-gateway/src/github.rs`, mod `tests`)

Deux assertions, et elles ne disent pas la même chose :

1. Dans `test_is_internal_repo_known_repos` (l. 2054-2061), `claude-pilot-py` →
   `claude-pilot`. C'est le test qui rougit le jour où la liste dérive de nouveau
   — la prescription explicite du doc de solution (« Écrire un test qui épingle
   le **nom courant** dans la population »).
2. Dans `test_is_internal_repo_unknown_repos` (l. 2064-2069), une assertion
   négative sur `senara-solutions/claude-pilot-py`. Elle épingle **D1**, pas un
   comportement accidentel : sans elle, quelqu'un rajoute l'alias « par
   symétrie avec `MPC_GATE_REQUIRED_REPOS` » et rien ne rougit. Son message
   d'assertion nomme la raison (liste autorisante) pour que le rouge soit
   lisible sans ouvrir ce plan.

### L3 — Les trois sites de prose qui décrivent l'état courant

| fichier | ce qui change |
|---|---|
| `skills/bundled/qa-review-webhook-success/system_prompt.md:21` | `claude-pilot-py` → `claude-pilot` dans les reviewable repos (D3) |
| `crates/mika-agent/src/server/ci_failure_handler.rs:591-592, 1386` | doc-comment + fixture `NOT_DISPATCHABLE` (D4) |
| `crates/mika-common/src/forge_identity.rs:307-312` | la parenthèse de justification de l'alias est amendée (D2) |

### L4 — Les docs (hors comptage)

`crates/mika-gateway/CLAUDE.md:37` (la liste énumérée),
`docs/architecture/mika-dev-work-assignment.md` (le tableau § 2 et la sonde S4
§ 8), `docs/agent-internals/tools.md:1648`,
`docs/operator/environment-variables.md:5290` (la section « ce que ce travail
n'achète PAS » de mika#2617, dont le constat est précisément celui que ce vol
lève — à amender avec la rectification de prémisse, pas à supprimer), et
`docs/solutions/security-issues/repo-name-keyed-guard-goes-inert-on-rename-keep-the-alias-2026-10-05.md:27`
(la parenthèse « `INTERNAL_REPOS` du gateway le liste encore », qui devient
datée).

**Aucun de ces fichiers n'est dans la liste `DOCS` de `scripts/sync-agent-docs.sh`**
(vérifié : `architecture.md`, `browser-control.md`, `configuration.md`,
`deployment.md`, `getting-started.md`, `runtime-structure.md`, `skills.md`,
`slash-commands.md`, `task-system.md`). Le job CI `docs-sync` n'est donc pas
déclenché et aucune copie crate-locale n'est à resynchroniser.

---

## Étapes d'implémentation

Un seul commit de travail suffit ; l'ordre ci-dessous est celui dans lequel les
tests restent interprétables si l'un rougit.

1. `github.rs` — la constante et son doc-comment (L1).
2. `github.rs` — les deux assertions (L2). `cargo test -p mika-gateway is_internal_repo`.
3. `forge_identity.rs` — la parenthèse amendée (D2). Vérifier que
   `mika2617_*` et `mpc_gate_required` restent verts et **inchangés** :
   l'alias MPC ne bouge pas.
4. `ci_failure_handler.rs` — doc-comment + fixture (D4).
   `cargo test -p mika-agent ci_failure` doit rester vert sans modification
   d'assertion.
5. `skills/bundled/qa-review-webhook-success/system_prompt.md` (D3), puis
   `make verify-bundled-skills`.
6. Les docs (L4).
7. Suite complète, `cargo clippy`, `cargo fmt --check`, les lints du dépôt,
   `scripts/verify-pipeline.sh origin/main`.

---

## Contrat de vérification

### W1 — le nom courant est dans la population

`cargo test -p mika-gateway test_is_internal_repo_known_repos`.
`is_internal_repo("senara-solutions/claude-pilot")` est vrai. C'est l'AC du
ticket, et c'est le test qui rougira au prochain renommage.

### W2 — l'ancien nom n'y est pas, et le rouge dit pourquoi

`cargo test -p mika-gateway test_is_internal_repo_unknown_repos`.
`is_internal_repo("senara-solutions/claude-pilot-py")` est faux. Discriminant :
réintroduire l'entrée fait rougir W2 alors que W1 reste vert — c'est exactement
la régression « alias rajouté par symétrie » que D1 refuse.

### W3 — l'alias MPC est intact

`cargo test -p mika-common mpc_gate` : `mpc_gate_required` reste vrai sur les
**deux** noms. Ce vol touche la prose de `forge_identity.rs`, jamais sa
population. Si cette assertion rougit, quelqu'un a appliqué D1 à la mauvaise
liste.

### W4 — le fixture migré ne déplace aucune assertion

`cargo test -p mika-agent ci_failure` vert **sans qu'aucune assertion ait
changé**. C'est la vérification de la prémisse de D4 (les deux noms sont hors
`DISPATCHABLE_REPOS`) : un rouge ici voudrait dire que `claude-pilot` est
dispatchable, ce que le code contredit, et le bon geste serait alors de revenir
au ticket, pas de retoucher le test.

### W5 — le bundle reste structurellement valide

`make verify-bundled-skills` (mika#1575) après L3.

### Gate Fire-Disposition (mika#2306) — N/A, et la raison plutôt que le silence

Ce plan ne livre **aucun détecteur**. Les deux assertions de W1/W2 sont des
assertions de comportement sur une fonction pure de six entrées : pas de
balayage d'arbre, pas de population de violations existantes à exempter, pas de
chemin de succès « aucune violation trouvée ». La section `## Fire-Disposition`
n'est donc pas requise et n'est pas inventée.

**Un scan structurel a été envisagé et écarté, avec sa raison.** Un détecteur du
type « aucune constante ne liste `claude-pilot-py` » serait **faux par
construction** : `MPC_GATE_REQUIRED_REPOS` porte cette entrée délibérément (D1,
D2), et ~20 occurrences de `docs/solutions/**` sont des faits datés (D5). Il
atterrirait donc avec une allowlist non vide dès le premier jour, pour garder une
constante de six entrées qu'un test couvre déjà. C'est de la machinerie pour un
coût — doctrine mika#2201.

---

## Risques nommés

- **R1 — la sonde du corps du ticket ne mesure pas ce qu'elle annonce.** `grep …
  'resolved via allowlist'` passera de vide à non-vide, mais cela n'atteste que
  « la liste correspond maintenant » : en mono-locataire les deux branches
  rendaient déjà la même route. Écrire cette sonde dans la PR sans sa limite
  serait publier un instrument qui ment. Voir § Mesure post-déploiement.
- **R2 — le changement de prompt n'est pas en service au merge.** `skills/bundled/`
  est une projection du binaire (mika#2340) ; sans `make deploy`, mika-qa
  continue de lire l'ancienne liste. À nommer dans le corps de PR.
- **R3 — l'alias MPC retiré par symétrie.** Le risque de régression le plus
  réaliste de ce vol : un lecteur qui applique D1 à `MPC_GATE_REQUIRED_REPOS`
  rouvre une porte de merge. W3 est la garde ; le doc-comment amendé de D2 est
  ce qui doit rendre la dissymétrie lisible **avant** qu'on la « corrige ».

---

## Hors périmètre

- Normaliser la casse / `trim` dans `is_internal_repo` (D5 — ce serait élargir
  une autorisation).
- `DISPATCHABLE_REPOS` : `claude-pilot` reste spawn-only (décision du 2026-08-29).
- Les occurrences `claude-pilot-py` désignant un chemin local, un paquet Python
  ou un fait daté dans `docs/solutions/**` (D5).
- Le routage multi-locataire des dépôts internes par lignes `github_repos`
  explicites : hors sujet, et déjà la doctrine du commentaire de code
  (`github.rs:1201-1208`).

---

## Mesure post-déploiement

**La sonde du corps du ticket est corrigée ici, conformément au commentaire de
grooming.**

1. **Ce que la sonde du corps mesure vraiment.**
   `grep 'senara-solutions/claude-pilot"' /var/log/mika/gateway.log | grep 'resolved via allowlist'`
   doit redevenir non vide au premier événement claude-pilot **après
   `make deploy`**. Lecture exacte : *la constante correspond de nouveau.* Ce
   n'est **pas** une preuve de routage — en mono-locataire, le fallback rendait
   déjà la même route au niveau `debug!`.
2. **La sonde qui mesure le routage** (celle du commentaire de grooming) : un
   tour du canal `github` sur un événement claude-pilot —
   `sessions.channel_type='github'` croisé avec un `tool_calls` portant le
   dépôt. Elle était déjà verte avant ce vol, et c'est précisément ce qui montre
   que le ticket est d'hygiène.
3. **Halte 1 — la ligne INFO reste vide alors que des PR claude-pilot bougent.**
   Vérifier d'abord le SHA déployé (`cat ~/.mika/skills/.manifest-writer`,
   `mika --version`) : une constante compilée n'atteint le gateway que par un
   rebuild. **Ensuite** seulement, vérifier que le webhook de l'App est installé
   sur le dépôt renommé.
4. **Halte 2 — la sonde 2 est vide.** Ne pas en conclure que ce vol a cassé le
   routage : il faut qu'un événement claude-pilot ait réellement eu lieu depuis
   le déploiement. *Une garde que personne n'a exercée se lit exactement comme
   une garde qui marche* (mika#2205).
5. **Pour D3 (le prompt)**, la sonde est distincte et vit sur le dépôt
   claude-pilot : une review mika-qa sur une PR claude-pilot après le prochain
   `make deploy`. Son absence avant le deploy est nominale (R2).

---

## Definition of Done

- `INTERNAL_REPOS` porte `senara-solutions/claude-pilot` et **pas** l'ancien nom.
- Le doc-comment de la constante porte D1 : liste autorisante, inversion de la
  règle d'alias, renvoi au doc de solution et à `MPC_GATE_REQUIRED_REPOS`.
- W1 et W2 verts ; W3 vert **sans modification** de la population MPC.
- Le fixture et les deux doc-comments de `ci_failure_handler.rs` nomment le nom
  courant ; `cargo test -p mika-agent ci_failure` vert sans changement
  d'assertion (W4).
- `qa-review-webhook-success/system_prompt.md` nomme le nom courant ;
  `make verify-bundled-skills` vert (W5).
- La parenthèse de `forge_identity.rs` ne cite plus `INTERNAL_REPOS` comme site
  périmé.
- Les cinq sites de documentation de L4 suivent.
- Suite complète, `cargo clippy`, `cargo fmt`, lints du dépôt et
  `scripts/verify-pipeline.sh origin/main` verts.
- Le corps de PR porte : la décision D1 **avec sa raison** (le ticket l'exige
  explicitement), la rectification de la prémisse « ne sont plus routés », la
  condition `make deploy` de R2, la sonde corrigée, et les sites `claude-pilot-py`
  laissés en place avec leur raison (D5).
- `Code review: delegated to mika-qa (mika#2694)` dans le corps de PR ; pas
  d'invocation de `ce-simplify-code` ni de `ce-code-review`.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria`. Les
critères ci-dessous sont dérivés de son § *Correctif attendu*, de son § *Mesure
post-déploiement* et du commentaire de grooming du 2026-10-07.

- **AC1** — `INTERNAL_REPOS` contient `"senara-solutions/claude-pilot"`.
- **AC2** — `is_internal_repo("senara-solutions/claude-pilot")` est vrai, épinglé
  par un test (W1).
- **AC3** — le test existant `github.rs:2058` ne teste plus l'ancien nom.
- **AC4** — la question de l'alias est **tranchée et écrite** : l'ancien nom est
  retiré, la raison est au site (doc-comment) et dans le corps de PR.
- **AC5** — `claude-pilot` reste absent de `DISPATCHABLE_REPOS` : ce vol rétablit
  le routage, jamais le dispatch.
- **AC6** — la mesure post-déploiement publiée dit ce que la sonde `grep … 'resolved
  via allowlist'` mesure **et ce qu'elle ne mesure pas**, et nomme la sonde du
  canal `github` comme celle qui mesure le routage.
- **AC7** — les autres sites portant l'ancien nom sont nommés dans la PR, avec,
  pour chacun, soit sa migration, soit la raison de ne pas le toucher.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-gateway/src/github.rs` (constante + doc-comment) | 14 |
| `crates/mika-gateway/src/github.rs` (deux assertions, mod `tests`) | 10 |
| `crates/mika-agent/src/server/ci_failure_handler.rs` (doc-comment + fixture) | 4 |
| `crates/mika-common/src/forge_identity.rs` (doc-comment amendé) | 5 |
| `skills/bundled/qa-review-webhook-success/system_prompt.md` | 1 |

Total estimé : 34 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Un seul vol, et il tient largement.** 34 lignes contre une borne de 243 et un
seuil par PR de 1000 : aucun découpage n'est requis, et ce n'est pas une
estimation optimiste d'un travail mal cerné — les cinq sites sont énumérés par
numéro de ligne, aucun n'introduit de structure nouvelle (pas de type, pas de
fonction, pas de table, pas de migration), et le changement le plus volumineux
est un paragraphe de doc-comment. Le coût de ce vol est dans la **lecture** qui
précède — déjà payée par ce grooming — pas dans l'écriture.

Les cinq fichiers de documentation de L4 ne sont pas comptés (`docs/` et
`CLAUDE.md`, par la règle). Ils représentent une dizaine de lignes de prose et
aucun n'entraîne de resynchronisation `docs-sync`.
