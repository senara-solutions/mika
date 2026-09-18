# mika#2024 — une remédiation prescrite à l'utilisateur ne peut pas supposer un terminal

**Ticket :** senara-solutions/mika#2024
**Type :** fix
**Date :** 2026-09-18

---

## Le défaut mesuré

Parcours champion Vincent, 2026-08-28 17:13. **Premier message** de l'agent,
verbatim :

> « You can fix that by running `gws auth login` from your terminal »

Un tenant cloud n'a aucun terminal. Pour un testeur externe non technique le
conseil n'est pas seulement inapplicable, il est illisible (« c'est quoi un
terminal ? »).

La source est écrite deux fois, à l'impératif, dans
`crates/mika-agent/templates/skills/google-workspace/system_prompt.md` :

- ligne 63 — « 2: Authentication error … **Ask the user to run `gws auth login`**
  to re-authenticate. »
- ligne 74 — « If `run_gws` reports an authentication error (exit code 2), tell
  the user their credentials may be expired and **suggest running
  `gws auth login`** to re-authenticate. »

---

## Ce que la lecture du code déplace — quatre mesures, dont une qui décide du plan

### M1 — la skill est `always_on`, et l'incident est un **premier message**

`templates/skills/google-workspace/skill.toml` pose `always_on = true`. Le
prompt est donc injecté dans le système de **tout** agent qui l'a en allowlist,
à **chaque** tour, indépendamment de tout mot-clé et de tout appel d'outil.

C'est la mesure qui décide de la forme du correctif. Le symptôme rapporté est un
*premier* message : **aucun appel `run_gws` en échec ne l'a précédé.** Le modèle
n'a pas réagi à un code de sortie — il a récité une ligne de son prompt système.

**Conséquence directe : un conditionnement posé uniquement au site du tool
result ne ferme pas le cas mesuré.** Il fermerait le cas « erreur d'auth
réellement constatée », qui n'est pas celui de l'incident. Toute conception qui
ne touche pas au texte du prompt laisse le mur exactement où il est.

### M2 — `Deployment` existe déjà, et son état majoritaire est `Unknown`

mika#2290 a livré `mika_common::home::Deployment` (`Local` | `Cloud` |
`Unknown`), résolu **à un seul site** (`server::init_agent`, gardé par
`mika2290_deployment_is_resolved_at_exactly_one_production_site`), caché sur
`AgentState.deployment` et threadé jusqu'à `ToolContext.deployment`
(`tools/mod.rs:116`). Rien n'est à construire.

**Mais aucun tenant cloud n'émet `MIKA_DEPLOYMENT` aujourd'hui** — le ticket
compagnon est `mika-cloud`, non livré, et le CLAUDE.md comme
`tests/eval/doctrine_regressions/false_local_hosting_claim_caught.rs` le disent
mot pour mot. **Le tenant champion du 28/08 était donc en `Unknown`, pas en
`Cloud`.**

C'est le piège central de ce ticket. Un prédicat `if deployment == Cloud`
n'aurait **rien fermé du tout** : il serait faux sur exactement la population
sinistrée. Le prédicat doit être **`Local` seul autorise une prescription de
terminal** — le reste ne la reçoit pas. C'est très précisément la forme que la
garde 5d de mika#2290 a déjà retenue, et pour la même raison écrite noir sur
blanc : *« a cloud tenant today carries no `MIKA_DEPLOYMENT`, resolves
`Unknown`, is therefore not `Local`, and the measured claim is refused from this
deploy onwards. »*

### M3 — `run_gws` ignore son `ToolContext`

`skills/builtin_handlers.rs:3324` : `async fn run_gws(input: &…, _ctx:
&ToolContext<'_>)`. Le contexte est reçu et jeté, alors qu'il porte déjà
`deployment` **et** `tier`. L'ancrage structurel est disponible sans changer
aucune signature.

### M4 — le balayage AC4 rend un positif, pas une confirmation

Le ticket demandait un balayage « de confirmation ». Il rend un **second cas de
la même classe** :

`templates/skills/browser-control/system_prompt.md:3-9` — « If no browser tools
are listed …, **tell the user** : … Run `mika mcp add playwright …` **Then
restart Mika.** » Une commande shell *et* un redémarrage de service, prescrits à
l'utilisateur. `browser-control` est dans `FAMILY_AGENT_SKILL_ALLOWLIST`
(`home.rs:665`), donc dans le même bassin famille-cloud que `google-workspace`.

Les autres occurrences relevées au balayage sont des **faux positifs à écarter
explicitement**, et la distinction porte tout le prédicat de la garde :

| Occurrence | Destinataire | Verdict |
|---|---|---|
| `google-workspace:63,74` — `gws auth login` | **l'utilisateur** | défaut |
| `browser-control:5-9` — `mika mcp add` + restart | **l'utilisateur** | défaut |
| `mcp:49` — « Run `mika mcp list` to verify » | l'agent | légitime |
| `github:41` — « Run `label list` once per conversation » | l'agent | légitime |
| `shell-exec:59` — « Run `shellcheck <script>` » | l'agent | légitime |

L'agent, lui, **a** un shell. Le prédicat n'est donc pas « mentionne une
commande » mais « prescrit à l'**utilisateur** une commande ». Une garde qui
confondrait les deux crierait sur trois prompts sains et serait désarmée dans la
semaine.

`skills/bundled/` rend **zéro** hit : ce sont les skills du loop autonome, dont
le destinataire est une session pilote, jamais un humain sans terminal.

---

## Décision de conception

### Les trois pistes, et pourquoi deux sont écartées

**Piste A — variante de prompt par runtime** (« variante de prompt par runtime »,
AC3). Écartée sur deux coûts. (1) `SkillPromptMap::resolve_prompt`
(`skills/index.rs:206`) est une chaîne à quatre étages indexée `provider/model` ;
un troisième axe multiplie le produit cartésien, fait bouger
`PromptVariantSource`, `PromptSource`, `scan_provider_variants` et leurs tests
structurels — pour une ligne de texte. (2) Plus grave : **ça reste de
l'enforcement par prompt.** `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
mesure ce que ça vaut (9 récurrences sous prompt contre 0 quand le substrat
tient), et l'AC3 demande explicitement de ne pas s'en remettre au fait que le
modèle devine.

**Piste B — capability-drop au boot** (façon `apply_load_safety_check`,
option 2 du ticket : « ne pas mentionner Calendar/Gmail du tout »). Écartée sur
une mesure : **`gws` est installé dans l'image cloud** (`Dockerfile.agent:70-75`,
téléchargé, sha256-vérifié, posé dans `/usr/local/bin/gws`). La skill n'est donc
pas structurellement inutilisable en cloud — elle est *inauthentifiable par le
geste proposé*. Évincer la skill retirerait Gmail, Calendar et Drive à tout
tenant non-`Local`, y compris ceux dont le provisionneur monterait des
credentials. **C'est une décision produit** (retirer une capacité annoncée),
irréversible sans nouveau ticket, et que ce ticket substrat n'a pas mandat de
prendre. Elle reste disponible si le produit la veut : la sonde (c) ci-dessous
est ce qui l'instruirait.

**Piste C — retenue : les deux moitiés, intent + enforcement.** C'est la doctrine
déjà écrite pour mika#2290 (« *the prompt is the intent half; the guard is what
closes the p1* ») et le seul montage que M1 rende suffisant.

### La forme retenue

**Moitié A — le prompt cesse de prescrire.** La remédiation disparaît du texte.
Les lignes 63 et 74 ne nomment plus aucun geste : elles décrivent la condition
(« les identifiants sont expirés ou invalides ») et renvoient explicitement à ce
que le résultat de l'outil dira. **C'est cette moitié, et elle seule, qui ferme
le cas mesuré** (M1 : un premier message ne suit aucun appel). Ce n'est pas « une
phrase de prompt en plus qui espère que le modèle devine » — c'est une phrase en
**moins** : on retire la source du récitatif au lieu d'ajouter une exception
qu'il faudrait que le modèle applique.

**Moitié B — le handler pose la remédiation, conditionnée au substrat.**
`run_gws` cesse d'ignorer son contexte. Sur code de sortie 2, il **annexe** au
tool result la remédiation réalisable par le destinataire réel, choisie par un
`match` exhaustif sur `(ctx.deployment, ctx.tier)`. Le modèle n'a alors plus
aucune autre source sur la question : le prompt se tait, le tool result parle.
C'est strictement plus structurel qu'une variante de prompt — le texte n'est pas
une consigne à suivre, c'est une donnée reçue.

### Le croisement `deployment × persona`, et pourquoi il est obligatoire

`FAMILY_SOUL` interdit « toute mention … de l'infrastructure sous-jacente —
jamais, même si on te le demande ». Un texte cloud qui parlerait de tenant, de
console ou de conteneur casserait la persona que Vincent a approuvée ; n'en
poser aucun laisserait le vide qui a produit l'incident. mika#2290 a tranché
exactement ce dilemme pour la ligne de hosting, avec un `match` exhaustif
`persona × deployment` et **aucun bras `_ =>`** (modèle nommé dans le CLAUDE.md :
`tools/mod.rs::dispatch_substrate_diagnostic`). Ce plan reprend cette forme
telle quelle.

Trois états côté deployment, et les bras sont nommés **même quand deux d'entre
eux font la même chose aujourd'hui** :

| `deployment` | Remédiation posée |
|---|---|
| `Local` | le geste terminal, tel quel — c'est la seule population qui a un terminal |
| `Cloud` | formulation sans terminal ; réservée au jour où le provisionneur émettra `cloud` |
| `Unknown` | **la population sinistrée d'aujourd'hui** — formulation sans terminal |

`Cloud` et `Unknown` convergent au déploiement. Les séparer quand même a un
coût nul et un bénéfice daté : le jour où `mika-cloud` émet la variable, poser un
lien console pour `Cloud` sans le poser pour `Unknown` devient un diff d'un bras.
Un `_ =>` aurait absorbé cette décision sans que personne la prenne.

**Ce que ça achète sans le ticket compagnon :** tout, pour le p2. Le tenant de
l'incident résout `Unknown`, n'est donc pas `Local`, et cesse de recevoir la
prescription **dès ce déploiement**. Le signal `cloud` améliore la *formulation*,
il n'a jamais été nécessaire au *refus*.

### Détection du code 2 — le piège de préfixe

`spawn_and_collect` formate `"Exit code: {code}\n{stderr}\n{stdout}"`
(`builtin_handlers.rs:793`), et le reste du fichier teste déjà ce préfixe
(lignes 847, 1819, 2549). Le post-traitement dans `run_gws` suit ce motif maison
plutôt que d'inventer un canal parallèle.

**Un `starts_with("Exit code: 2")` nu est faux** : il matche `Exit code: 23` et
`Exit code: 25`. C'est exactement le piège `#234` / `#2343` que mika#2347 a dû
fermer sur une autre surface. Le prédicat exige donc que le `2` soit suivi d'une
fin de ligne ou d'une fin de chaîne. À défaut de correspondance, **on n'annexe
rien** : un code non reconnu n'est jamais traité comme une erreur d'auth.

### Ce qui n'est délibérément PAS fait : pas de garde EndTurn

Une garde de sortie (famille `guard.*`, modèle 5d) refuserait un tour dont le
texte prescrit un geste terminal hors `Local`. Elle n'est **pas** posée ici, et
c'est un choix argumenté, pas une omission :

- la source du récitatif est retirée (moitié A), donc il n'y a plus de texte à
  réciter — poser la garde reviendrait à se prémunir contre une invention pure du
  modèle, ce qu'aucune mesure ne documente sur cette surface ;
- une garde de sortie a un coût permanent (faux positifs sur le tenant `Local`
  légitime, où la phrase est **correcte**), et un prédicat textuel qui doit
  distinguer « prescrire à l'utilisateur » de « mentionner » est précisément
  celui dont M4 montre qu'il est difficile ;
- le ticket est un p2 sur une surface non critique.

La sonde (b) ci-dessous est ce qui dirait qu'elle est devenue nécessaire. **Si
elle rend un positif, la garde est le suivi, pas un élargissement de la
moitié B.**

---

## Requirements

### R1 — le prompt `google-workspace` ne prescrit plus aucun geste
Lignes 63 et 74 réécrites : la condition est décrite, le geste ne l'est plus, le
renvoi au tool result est explicite. Aucune occurrence de `gws auth login` ne
subsiste dans le fichier.

### R2 — `run_gws` annexe la remédiation, conditionnée au substrat
Sur sortie 2 exactement, `run_gws` annexe un texte choisi par un `match`
exhaustif `(Deployment, PersonaProfile-équivalent via ctx.tier)`, sans bras
`_ =>`. Les autres codes de sortie sont inchangés, octet pour octet.

### R3 — `Local` est le seul état qui reçoit un geste terminal
`Cloud` et `Unknown` reçoivent une formulation réalisable sans terminal. Bras
nommés séparément.

### R4 — deux registres, l'infrastructure hors de la persona famille
`AgentTier::{Family, Champion}` reçoivent une formulation sans jargon
d'infrastructure ; `Default` reçoit la formulation opérateur. `Champion` est
nommé dans son bras (précédent mika#2023 AC5), jamais absorbé par un catch-all.

### R5 — `browser-control` est neutralisé dans le même PR
Le bloc lignes 3-9 cesse de prescrire `mika mcp add …` + « restart Mika » à
l'utilisateur. Il décrit l'indisponibilité sans geste, la mise en place étant
affaire d'opérateur.
**Sans ancrage structurel** : il n'y a pas de handler Rust équivalent (la
capacité est fournie par MCP, pas par un builtin), donc cette moitié est
prompt-seule et le plan le dit plutôt que de le maquiller. C'est la garde R6 qui
empêche la réintroduction.

### R6 — une garde permanente refuse la réintroduction
Scan de source sur les `system_prompt.md`, refusant une prescription de geste
**adressée à l'utilisateur**. Motif du dépôt :
`grooming_marker::tests::no_grooming_regex_outside_this_module` et
`auto_pull::tests::mika2131_exclusion_skips_never_return_to_an_uncollected_debug`.

**Pourquoi un test comportemental ne suffit pas :** la régression consisterait à
réécrire la ligne dans le prompt. Aucune assertion sur `run_gws` ne rougirait —
le handler continuerait de faire exactement ce qu'on lui demande, pendant que le
prompt reprendrait la parole. C'est la même raison qui a fait écrire la garde
mika#2205 : le défaut ne rendrait pas une décision fausse, il la rendrait
inopérante.

Le prédicat vise la **prescription** (« ask/tell the user to run », « suggest
running », « from your terminal ») et **pas** la mention (« Run `label list` »,
adressé à l'agent). Contrôle de bonne foi obligatoire : la garde doit être
vérifiée non vacue (elle trouve bien les fichiers) **et** ne pas rougir sur les
trois occurrences légitimes de M4.

### R7 — observabilité
Un événement au site d'annexion, pour que la population soit comptable. Nom
dédié, écrit à un seul site.

---

## Implementation

### Fichiers touchés

| Fichier | Nature |
|---|---|
| `crates/mika-agent/templates/skills/google-workspace/system_prompt.md` | R1 |
| `crates/mika-agent/templates/skills/browser-control/system_prompt.md` | R5 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` | R2, R3, R4, R7 + tests |
| `crates/mika-agent/tests/` (nouveau fichier de garde) | R6 |

### Étapes

1. **R1** — réécrire les lignes 63 et 74. La ligne 63 (table des codes de
   sortie) garde sa valeur descriptive et perd son impératif. La ligne 74
   (Guidelines) renvoie au contenu du tool result plutôt que de nommer un geste.
   Vérifier `grep -c "gws auth login"` → 0.

2. **R2/R3/R4** — dans `run_gws` :
   - renommer `_ctx` en `ctx` ;
   - après `spawn_and_collect`, tester le code 2 avec le prédicat borné
     (fin de ligne / fin de chaîne après le `2`) ;
   - construire la remédiation par un `match` exhaustif sur `ctx.deployment`,
     puis sur `ctx.tier`, sans bras `_ =>` ;
   - annexer au `content` avec un séparateur ligne vide (motif de
     `dispatch_substrate_diagnostic`), sans toucher `is_error`.

   La fonction de sélection du texte est extraite en fonction **pure**
   (`(Deployment, AgentTier) -> &'static str`) : c'est ce qui rend les six
   combinaisons testables sans processus ni `gws` installé.

3. **R5** — réécrire le bloc `browser-control:3-9`.

4. **R6** — garde de scan de source, avec son contrôle de non-vacuité et ses
   trois cas légitimes en contrôle négatif.

5. **R7** — `info!` au site d'annexion, champs `deployment`, `tier`, et rien qui
   puisse porter un identifiant de compte Google.

### Ce qui ne bouge pas

`bundled_skills.rs` embarque le prompt par `include_str!` : le fichier édité est
déjà celui qui est compilé, aucune déclaration à ajouter. La chaîne
**rebuild → seed → read** du CLAUDE.md § *Deploying a bundled-skill change*
s'applique telle quelle — l'édition n'atteint aucun agent avant `make deploy`.

---

## Verification contract

### Tests unitaires (fonction pure de sélection)
- les six combinaisons `{Local, Cloud, Unknown} × {Default, Family}` rendent le
  texte attendu ;
- `Local` est le **seul** état dont le texte contient `gws auth login` ;
- `Cloud` et `Unknown` rendent un texte identiquement dépourvu de toute mention
  de terminal, de shell et de commande ;
- `Family` et `Champion` rendent un texte dépourvu de jargon d'infrastructure ;
- contrôle négatif structurel : le `match` n'a pas de bras `_ =>` (scan de
  source sur la fonction), sans quoi un futur variant hériterait d'une décision
  que personne n'a prise pour lui.

### Tests du prédicat de code de sortie
- `"Exit code: 2"` et `"Exit code: 2\n…"` → annexe ;
- `"Exit code: 23"`, `"Exit code: 25"` → **n'annexe pas** (le piège de préfixe,
  posé en test et pas seulement en commentaire) ;
- `"Exit code: 1"`, `"Exit code: 3"`, sortie réussie → contenu inchangé octet
  pour octet.

### Tests de prompt (R1, R5)
- `google-workspace/system_prompt.md` ne contient plus `gws auth login` ;
- `browser-control/system_prompt.md` ne prescrit plus de commande à
  l'utilisateur.
Assertions portées sur le contenu embarqué (`include_str!`), pas sur un chemin
disque : c'est l'octet compilé qui atteint l'agent.

### Garde permanente (R6)
- rouge si une prescription utilisateur réapparaît dans un `system_prompt.md` ;
- verte sur `mcp:49`, `github:41`, `shell-exec:59` (contrôle négatif — le
  prédicat distingue le destinataire) ;
- non vacue (trouve bien les fichiers scannés).

### Commandes
```
cargo test -p mika-agent
cargo clippy --all-targets -- -D warnings
cargo fmt --check
make verify-bundled-skills
```

---

## Fire-Disposition

*(Exigée par le Fire-Disposition Gate — mika#1574. Ce plan livre des deliverables
de classe détecteur ; cette section dit comment chacun se comporte face aux
données **préexistantes**.)*

**Disposition retenue : (c) halt-and-surface. Allowlist vide, aucun détecteur
livré désarmé.**

### Trois détecteurs, pas un

L'architecte nomme R6. Le plan en livre trois, et les trois sont traités ici
plutôt qu'un seul :

| Détecteur | Portée sur le préexistant |
|---|---|
| **R6** — scan de source sur les `system_prompt.md` | **le seul qui tire sur du préexistant** |
| **Assertions de contenu de prompt** (R1, R5 — § *Tests de prompt*) | tirent sur les octets que ce PR édite |
| **Scan structurel « pas de bras `_ =>` »** (§ *Tests unitaires*, contrôle négatif) | porte sur la fonction que ce PR écrit — périmètre préexistant nul |

### R6 — l'état est vérifié avant l'écriture, pas espéré

*État à l'écriture du plan*, mesuré sur les deux arbres scannés (11
`system_prompt.md` sous `crates/mika-agent/templates/skills/`, 25 sous
`skills/bundled/`) :

- **deux** violations, toutes deux dans `templates/skills/` — `google-workspace`
  l63 et l74, `browser-control` l5–9 ;
- **zéro** dans `skills/bundled/`, ce que § M4 avait déjà relevé ;
- les deux sont retirées par **R1 et R5, dans ce même PR**.

Donc à l'instant du land, le détecteur passe au vert sans qu'aucune ligne ne soit
tolérée *pour lui* : les seules lignes qu'il aurait attrapées sont précisément
celles que le correctif supprime. **C'est cette coïncidence — le périmètre du
détecteur est exactement le périmètre du fix — qui rend (c) disponible ici**, et
elle est vérifiée par décompte avant l'écriture. Sans elle, (c) aurait été une
prudence déguisée en décision.

*Allowlist : vide, et la vacuité est l'invariant.* Les trois occurrences
légitimes de § M4 (`mcp:49`, `github:41`, `shell-exec:59`) ne sont **pas** des
exemptions : elles sont hors prédicat par construction — leur destinataire est
l'agent, qui a un shell. Elles vivent dans le test comme **contrôle négatif**
(le prédicat doit rester vert dessus), jamais comme entrées tolérées. La
distinction n'est pas cosmétique : une allowlist qui les nommerait dirait que le
prédicat les attrape et qu'on ferme les yeux, alors que le plan tout entier
repose sur le fait qu'il ne les attrape pas (§ M4 : « une garde qui confondrait
les deux crierait sur trois prompts sains et serait désarmée dans la semaine »).

### Si R6 tire au land time — la halte, et ce qu'elle interdit

Un troisième positif signifierait que le balayage AC4 de § M4 est **incomplet**,
donc que la table destinataire qui porte tout le prédicat a manqué une forme
d'écriture. Le remède **n'est ni d'ajouter le fichier à un allowlist, ni
d'assouplir le prédicat** : c'est de lire l'occurrence, de reprendre § M4, et de
décider si elle relève de R1/R5 (retrait dans ce PR) ou d'un suivi. Un
assouplissement rendrait le détecteur vert en lui retirant exactement la capacité
pour laquelle il est livré.

Note de conception à l'appui : les deux violations connues sont écrites sous
**deux formes différentes** — impératif inline (« Ask the user to run … ») et
adresse suivie d'un bloc de commande (« tell the user : … » + fence). Le
prédicat doit couvrir les deux, ce qui est aussi la raison pour laquelle une
troisième forme est plausible et pour laquelle la halte est écrite plutôt que
supposée impossible.

### Auto-comptage : pas d'exclusion nécessaire, et c'est une propriété du périmètre

Le fichier de test R6 contiendra **littéralement** les chaînes proscrites (ses
contrôles négatifs et ses cas rouges). Il ne se compte pourtant pas lui-même,
parce que le scan énumère des `system_prompt.md` par glob sur deux arbres de
prompts — et non `src/`, comme le faisait le T7 de mika#2361, qui a dû
s'auto-exclure pour cette raison exacte.

**Cette absence d'exclusion est une conséquence du périmètre, pas une garantie
du prédicat.** Élargir un jour le scan à `src/` ou à `tests/` ferait
silencieusement rougir la garde sur ses propres fixtures. Le test doit donc
épingler son périmètre (les deux globs), pas seulement son verdict — sans quoi
l'élargissement se lirait comme une découverte.

### Pourquoi ni (a) ni (b)

- **(a) allowlist nommée** — écartée : il n'y a rien à exempter (le décompte
  ci-dessus rend zéro après R1/R5). Une entrée d'allowlist légitimerait une
  prescription utilisateur survivante, c'est-à-dire précisément la classe que ce
  ticket ferme, et elle survivrait au suivi censé la retirer.
- **(b) land disabled** — écartée, et c'est l'argument le plus fort du plan
  contre elle : **R5 est prompt-seul** (§ R5 : « pas de handler Rust équivalent
  … c'est la garde R6 qui empêche la réintroduction »). Livrer R6 désarmé
  laisserait la moitié `browser-control` du correctif sans aucune protection
  anti-régression — le détecteur désarmé n'est pas un moindre mal ici, c'est la
  suppression de la seule enforcement de R5.

---

## Definition of Done

- [ ] `grep -c "gws auth login" crates/mika-agent/templates/skills/google-workspace/system_prompt.md` rend `0`.
- [ ] `run_gws` lit `ctx` ; le `match` de sélection est exhaustif et sans `_ =>`.
- [ ] Les six combinaisons `deployment × tier` sont couvertes par un test.
- [ ] Le prédicat de code 2 refuse `Exit code: 23`, en test.
- [ ] `browser-control` ne prescrit plus de commande à l'utilisateur.
- [ ] La garde R6 est verte, non vacue, et ne rougit pas sur les trois cas légitimes.
- [ ] La garde R6 est verte **sans aucune entrée d'allowlist** (§ *Fire-Disposition* : la vacuité est l'invariant). Un positif au land time est une **halte**, pas une exemption.
- [ ] Le test R6 épingle son périmètre (les deux globs `templates/skills/**/system_prompt.md` et `skills/bundled/**/system_prompt.md`), et pas seulement son verdict.
- [ ] `cargo test -p mika-agent`, `clippy -D warnings`, `fmt --check`, `make verify-bundled-skills` passent.
- [ ] Aucun test existant n'est affaibli ou supprimé pour faire passer ce travail.

---

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2024 :

- [ ] Sur erreur d'authentification `run_gws` dans un runtime sans terminal, l'agent ne propose plus `gws auth login`.
- [ ] La remédiation proposée est réalisable par le destinataire réel (lien console, ou aucune mention de la capacité).
- [ ] Le conditionnement est **structurel**, pas une phrase de prompt en plus qui espère que le modèle devine le substrat (cf. `feedback_prompt_enforcement_fragile`) — variante de prompt par runtime, ou capability-drop au boot façon `apply_load_safety_check`.
- [ ] Un balayage des autres prompts de skills bundled confirme qu'aucun autre ne prescrit une action locale-seulement. Candidats à vérifier au minimum : `shell-exec`, `tmux`, `git-ops`, `desktop`, `browser-control`.

**Note de traçabilité sur AC3.** La lettre propose deux formes (« variante de
prompt par runtime, ou capability-drop au boot »). Ce plan retient une
**troisième**, argumentée en § *Décision de conception* : le retrait de la
prescription du prompt (une phrase en moins, pas en plus) plus un conditionnement
au site d'émission du tool result. Elle satisfait l'exigence de fond — le
conditionnement ne repose pas sur une devinette du modèle, il est porté par un
`match` exhaustif sur une valeur résolue au boot — et elle est la seule des trois
que M1 rende suffisante. La forme capability-drop reste disponible, et la sonde
(c) est ce qui l'instruirait.

**Note sur AC4.** Le balayage ne « confirme » pas : il rend un positif
(`browser-control`), traité en R5, plus trois faux positifs écartés avec leur
critère en § M4. `desktop` et `calendar`, nommés parmi les candidats, **n'existent
pas** — voir *Hors périmètre*.

---

## Hors périmètre, délibérément

- **Le ticket frère mika#2023** (tier champion : persona + allowlist opérateur).
  Cause distincte, même parcours ; ce défaut tient même une fois #2023 corrigé,
  et le commentaire 1/2 du ticket le dit.
- **Le ticket compagnon `mika-cloud`** qui émettra `MIKA_DEPLOYMENT=cloud`.
  L'ordre est contraint et `mika` passe en premier : ce correctif ferme le p2 sur
  `Unknown`, donc il ne l'attend pas. L'émission améliore la formulation
  (`Cloud`), pas le refus.
- **Le flux OAuth Google via la console** (voie 1 du ticket). Il n'existe pas ;
  ce plan n'en invente pas l'URL. Le texte `Cloud` reste factuel — dire « je ne
  peux pas rétablir cet accès depuis ici » est vrai, alors qu'un lien fabriqué
  serait une seconde impasse pour le même champion. Le jour où la console porte
  le flux, c'est un diff d'un bras.
- **Le capability-drop de `google-workspace` en cloud** (voie 2). Décision
  produit, instruite par la sonde (c).
- **Une garde EndTurn.** Argumenté en § *Décision de conception* ; conditionné à
  la sonde (b).
- **`desktop` et `calendar`, entrées fantômes de `FAMILY_AGENT_SKILL_ALLOWLIST`.**
  Trouvé en chemin : `home.rs:665` allowliste six skills, dont deux qui
  n'existent ni dans `templates/skills/` ni dans `skills/bundled/`. Sans effet
  connu (une allowlist nomme, elle ne crée pas), mais une allowlist qui nomme des
  skills inexistantes est une allowlist qu'on ne peut plus lire comme une
  mesure de surface. **Ticket de suivi à ouvrir**, sans rapport avec la
  remédiation.

---

## Sondes post-déploiement, et leurs haltes

Rappel : rien n'atteint un agent avant `make deploy` (chaîne **rebuild → seed →
read**). Vérifier `cat ~/.mika/skills/.manifest-writer` avant toute conclusion —
une sonde lue sur une library pré-deploy ne mesure rien.

**(a) Symptôme, rejouable immédiatement.** Sur un tenant non-`Local`, provoquer
une erreur d'auth `run_gws` et relire la réponse : aucune mention de terminal, de
shell, ni de `gws auth login`. Puis, sans provoquer d'erreur, ouvrir une
conversation et demander l'agenda — c'est le cas de l'incident (premier message).
**Halte :** si la prescription réapparaît alors que le prompt ne la contient
plus, la moitié A n'a pas atteint l'agent — lire `.manifest-writer` **avant** de
toucher au code.

**(b) La garde est-elle devenue nécessaire.** Sur 48 h, aucune réponse d'un
tenant non-`Local` ne doit prescrire un geste terminal. Un positif signifie que
le modèle invente la remédiation sans l'avoir lue : c'est **là** que la garde
EndTurn se justifie, et pas avant. **Ne pas élargir la moitié B pour ça** — elle
ne s'exécute que sur un appel d'outil, et une invention pure n'en passe par
aucun.

**(c) La capacité est-elle atteignable du tout en cloud.** Compter les appels
`run_gws` rendant 2 sur les tenants non-`Local`. **S'ils sont ~100 %**, la
skill est annoncée et inatteignable, et c'est l'argument chiffré de la voie 2 du
ticket (capability-drop) — le rendre au produit plutôt que de reformuler une
remédiation de plus. **S'ils sont marginaux**, les credentials sont montés
quelque part et le capability-drop aurait détruit une capacité qui marche : c'est
le contrôle négatif qui justifie a posteriori d'avoir écarté la piste B.

**(d) Régime attendu de l'événement R7 :** faible et non nul. Nul sur 48 h avec
des tenants actifs signifie que le site d'annexion n'est pas atteint — vérifier
le prédicat de code de sortie **avant** de conclure que le défaut a disparu (une
sonde silencieuse et une sonde saine se ressemblent, mika#2205).

---

## Risques

| Risque | Portée | Traitement |
|---|---|---|
| Un opérateur `Local` sans `MIKA_DEPLOYMENT` perd le geste terminal, qui était correct pour lui | Réel, assumé | Exactement le coût nommé par mika#2290 pour la ligne de hosting : une ligne dans `~/.mika/.env`. Ce que l'agent dit entre-temps reste vrai. Le sens de l'asymétrie est le bon — un opérateur sait lire « je ne peux pas rétablir ça depuis ici », un champion non technique ne sait pas ouvrir un terminal qui n'existe pas. |
| Le modèle reformule le texte annexé et réintroduit le geste | Faible | Le prompt ne contient plus rien à réintroduire. Mesuré par la sonde (b) ; la garde EndTurn est le suivi si elle rend un positif. |
| Le prédicat de la garde R6 dérive et crie sur des prompts sains | Moyen | Contrôle négatif obligatoire sur les trois occurrences de M4, dans le test lui-même. Une garde désarmée pour cause de faux positifs est pire que pas de garde. |
| `browser-control` reste prompt-seul | Assumé, écrit | Pas de handler Rust à conditionner (capacité MCP). R6 couvre la réintroduction — et c'est pourquoi § *Fire-Disposition* écarte l'option (b) : R6 désarmé laisserait R5 sans aucune enforcement. |

---

## Revision history

- **rev 2 (2026-09-18)** — adressé **F1** (BLOCKING, Fire-Disposition Gate,
  mika#1574) par l'ajout d'une section `## Fire-Disposition` retenant l'option
  **(c) halt-and-surface** avec allowlist vide. La disposition n'est pas posée par
  prudence mais sur un décompte vérifié avant l'écriture — 36 `system_prompt.md`
  scannés (11 `templates/skills/` + 25 `skills/bundled/`), exactement deux
  violations, toutes deux retirées par R1/R5 dans ce même PR : le périmètre du
  détecteur coïncide avec celui du fix, ce qui est la condition qui rend (c)
  disponible. Trois points sont allés au-delà de la lettre du finding, parce que
  les écrire coûtait moins que de les laisser au pilote : (1) l'inventaire nomme
  **trois** deliverables de classe détecteur et non le seul R6 (les assertions de
  contenu de prompt R1/R5 et le scan structurel « pas de bras `_ =>` » en sont
  aussi) ; (2) la question de l'auto-comptage est tranchée — le fichier de test R6
  contiendra littéralement les chaînes proscrites, et s'il ne se compte pas
  lui-même c'est une propriété du périmètre (glob sur des `system_prompt.md`, non
  sur `src/` comme le T7 de mika#2361 qui a dû s'auto-exclure), d'où l'obligation
  d'épingler le périmètre et pas seulement le verdict ; (3) le rejet de l'option
  (b) est argumenté sur le plan lui-même — R5 étant prompt-seul, livrer R6 désarmé
  ne serait pas un moindre mal mais la suppression de la seule enforcement de R5.
  Deux lignes ajoutées à la *Definition of Done* pour rendre la disposition
  vérifiable (allowlist vide ; périmètre épinglé). **Aucune AC affaiblie, aucun
  requirement retiré** : F1 était un manque de spécification, pas une objection de
  conception, et la Piste C reste intacte.
