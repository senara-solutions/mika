---
issue: 2178
repo: senara-solutions/mika
type: fix
module: skills/bundled/_shared/dispatch-lib.sh
tags: [dispatch, loop-substrate, pilot-context, grooming]
problem_type: missing-context-channel
status: groomed
---

# mika#2178 — le texte du ticket (corps ET commentaires) n'atteint jamais le pilote

**Issue :** senara-solutions/mika#2178
**Branche :** `fix/2178/dispatch-lib-le-contexte-du-pilote`
**Palier :** Tier 1 — casse la boucle silencieusement (un dispatch part avec une consigne
opérateur que personne ne lit, et rien ne le signale).

> Ce plan est la seconde rédaction. La première a été arrêtée en `ESCALATE-divergence` à la
> Phase 2.5 : elle contredisait la prémisse du corps d'alors (« ajouter `comments` à la ligne
> 1836 »). L'opérateur a tranché en re-scopant le corps le 2026-09-05 — point d'injection
> corrigé, périmètre élargi au corps. Ce plan suit le corps re-scopé ; il n'y a plus de
> divergence à arbitrer.

---

## 1. Le fait, mesuré sur `origin/main` @ `f965dbeb`

### 1.1 Ce que le pilote reçoit vraiment

`skills/bundled/_shared/dispatch-lib.sh:2080` :

```bash
# The prompt becomes the QUALIFIED issue reference.
# Must use `${REPO}#${ISSUE_NUM}` (not bare `#${ISSUE_NUM}`) — see mika#138.
PROMPT="${REPO}#${ISSUE_NUM}"
```

puis `:2182` :

```bash
_run_pilot_sandboxed claude-pilot ... --command "$ENTRY_COMMAND" $CWD_ARGS -- "$PROMPT"
```

et, côté pilote, `claude-pilot/src/claude_pilot/cli.py` assemble l'invite d'ouverture par
simple concaténation — `:290` en interactif, `:251` en headless :

```python
opening_prompt = f"{ns.command} {opening}"
```

L'entrée réelle du pilote est donc **`<ENTRY_COMMAND> <PROMPT>`**. Mesuré pour mika#2158 :
`PROMPT` vaut `mika#2158` (9 octets) ; l'invite d'ouverture vaut `/mika mika#2158`
(15 octets) sur le chemin sans callout. Rien du corps, rien des étiquettes, rien des
commentaires.

### 1.2 Les deux chemins, et pourquoi le plus courant est le plus aveugle

`_detect_plan_on_branch` (`:5271-5310`, appelé à `:5710`) réécrit l'entrée quand le corps
porte un callout `> - **Plan:** \`docs/plans/…\`` **et** que le fichier existe dans le
worktree :

```bash
ENTRY_COMMAND="/ce-work $PLAN_PATH"
```

| chemin | invite d'ouverture reçue | corps du ticket | commentaires |
|---|---|---|---|
| **A** — callout de plan présent | `/ce-work docs/plans/x.md mika#N` | jamais lu — le contrat est le fichier de plan | jamais |
| **B** — pas de callout | `/mika mika#N` | re-fetché par le pilote (`.claude/commands/mika.md:12`, `--json number,title,body,labels`) | jamais |

Le chemin A est celui qu'emprunte **tout ticket groomé** — exactement la population qui porte
des enrichissements de grooming.

### 1.3 La preuve : mika#2158, chemin A

Le corps de mika#2158 porte
`> - **Plan:** \`docs/plans/2026-09-03-001-fix-2158-un-seul-predicat-detat-de-grooming-plan.md\``,
fichier sur branche depuis `a183489d`. Le dispatch de `09:44:04Z` a donc reçu
`/ce-work docs/plans/2026-09-03-001-….md mika#2158` : ni le commentaire de `09:40Z`, ni le
corps.

**Conséquence qui a re-scopé ce ticket.** Le remède appliqué à #2158 à `10:55Z` — déplacer la
consigne du commentaire vers le corps — **n'a rien réparé** : sur le chemin A le corps n'est
pas plus lu que le commentaire. Le 5ᵉ pilote (12:50→17:29 local) est mort sur une chaîne
`cat > /tmp/p…`, la forme exacte que la consigne déplacée décrivait comme refusée.
Corroborant, pas décisif — un modèle peut lire un avertissement et le violer. Le fait
structurel, lui, est décisif : `ISSUE_BODY` n'est écrit dans aucune invite, sur aucun chemin.

### 1.4 Les sept lecteurs internes, et pourquoi ils ne comptent pas ici

`ISSUE_BODY` est lu à `:1865`, `:1873`, `:1891`, `:1902`, `:5284`, `:5294`, `:5790` —
dérivation de branche, porte anti-re-groom (mika#2012), détection du callout de plan,
sauvetage. **Tous internes à dispatch-lib.** Aucun n'écrit dans l'invite. C'est pourquoi
étendre `--json …,comments` à `:1836` *seul* remplit une variable shell que personne ne
transmet : un correctif qui ne rougit aucun test réel et ne change aucun octet de ce que voit
l'implémenteur. C'est la vacuité que **AC3** interdit.

### 1.5 Pourquoi le correctif vit dans dispatch-lib, pas dans `mika.md`

Réparer `.claude/commands/mika.md:12` (`--json …,comments`) ne couvre que le chemin B. Le
chemin A resterait aveugle. dispatch-lib est le seul composant qui (a) voit le ticket avant le
démarrage du pilote et (b) contrôle l'invite d'ouverture des deux chemins.

---

## 2. Périmètre

**Dans le périmètre.** `skills/bundled/_shared/dispatch-lib.sh` : récupération des
commentaires, rendu borné du corps et des commentaires, injection dans `PROMPT`. Tests dans
`skills/bundled/_shared/test-dispatch-lib.sh`.

**Hors périmètre, et pourquoi.**

- `.claude/commands/mika.md:12` — un opérateur qui lance `/mika #2178` **à la main** reste
  privé des commentaires. Réel, mais c'est une autre surface (interactive, pas dispatch) et un
  autre contrat. Ticket de suivi, pas un bundle (`feedback_implementation_scope_bundling`).
  Note : sur le chemin dispatch B, l'injection décrite ici rend le point sans objet — c'est
  seulement l'invocation manuelle qui reste découverte.
- `/mika-groom-plan-only` (entrée `dev-groom`) — l'injection le sert gratuitement, puisqu'elle
  s'applique au `PROMPT` avant le choix de `ENTRY_COMMAND` ; mais ce n'est pas ce que ce
  ticket promet, et aucun test ne l'asserte.
- Le classifieur de permissions et les formes de commande refusées — claude-pilot#151.
- Toute modification des sept lecteurs internes de `ISSUE_BODY` : **AC4 les gèle**.

---

## 3. Le correctif

### 3.1 Étendre la récupération — nécessaire, jamais suffisant

`:1836` :

```bash
ISSUE_JSON=$(gh issue view "$ISSUE_NUM" --repo "senara-solutions/$REPO" \
    --json state,title,labels,body,comments 2>/dev/null) || {
```

Un seul appel, pas un second. `state` est déjà lu ici pour la porte issue-close ; un second
aller-retour ouvrirait une fenêtre TOCTOU entre l'état et les commentaires — la même que la
spec de `/mika-groom-ticket` a fermée à son étape 5a.

Forme rendue par `gh` (vérifiée sur mika#2158) : `.comments[]` porte `.author.login`,
`.createdAt` (ISO 8601 Z) et `.body`.

### 3.2 `_render_ticket_context()` — le helper

Signature : lit `ISSUE_JSON` sur stdin, écrit sur stdout le bloc de contexte rendu ; **chaîne
vide** si ni corps ni commentaire à rendre.

Structure émise :

```
CONTEXTE DU TICKET (senara-solutions/mika#2178)
Le corps et les commentaires ci-dessous FONT PARTIE du ticket. Une consigne opérateur,
une correction de trajectoire ou un enrichissement de grooming y vit aussi souvent dans
un commentaire que dans le corps. Lis les deux avant d'agir.

--- corps du ticket ---
<corps>

--- commentaire 3/5 · samidarko (humain) · 2026-09-04T09:43:28Z ---
<corps du commentaire>
```

Trois décisions, chacune motivée :

- **L'en-tête dit quoi en faire**, pas seulement que le texte existe. Un bloc étiqueté
  « contexte » sans consigne de lecture se lit comme du bruit d'archive — c'est le précédent
  d'`ITERATION CONTEXT` (`:2085`), qui nomme son usage.
- **Rôle `bot` / `humain`** par login (`mika-platform-dev`, `github-actions` → `bot`).
  Un verdict QA `block[pipeline]` posté par `mika-platform-dev` et une consigne opérateur
  postée par un humain ne doivent pas se lire pareil ; l'auteur est le seul discriminant
  disponible sans heuristique sur le texte. Les deux logins observés sur mika#2158 sont
  `mika-platform-dev` et `samidarko` — la liste couvre le cas mesuré.
- **Séparateurs `---` en clair, pas de bloc de code.** Un commentaire contient couramment des
  clôtures ```` ``` ```` qui casseraient toute imbrication.

**Bornes, et leur raison écrite dans le code :**

| borne | valeur | raison |
|---|---|---|
| corps | 16 384 o, marqueur `[… corps tronqué à 16384 o]` | Les corps groomés mesurés font 3–8 Kio ; 16 Kio rend la troncature rare tout en bornant le pire cas. |
| nombre de commentaires | 10 derniers, rendus du plus ancien au plus récent | Une correction de trajectoire est récente par construction : elle réagit à un dispatch mort. La règle « postérieurs au dernier callout de grooming » a été écartée — le callout du corps ne porte **aucune date**, donc « postérieur » n'est pas calculable depuis le corps. Le flux de commentaires est la seule surface datée. |
| par commentaire | 4 096 o, marqueur `[… tronqué à 4096 o]` | Aligné sur le plafond existant d'`ITERATION_CTX` (`:2085`, `head -c 4096`). Le fichier garde **une** convention, pas deux. |
| bloc commentaires | 16 384 o, éviction du plus ancien d'abord, ligne `[N commentaire(s) plus ancien(s) omis]` | Dix commentaires à 4 Kio valent 40 Kio ; le plafond global rend le pire cas déterministe (≤ 32 Kio de contexte total avec le corps). L'éviction part du plus ancien parce que le plus récent porte le plus probablement la correction. |

**Toute omission est dite.** Un contexte silencieusement amputé est le défaut que ce ticket
répare, pas un défaut qu'il a le droit de réintroduire.

### 3.3 Injection dans `PROMPT` — le point qui porte

À `:2080-2086`, **après** la branche `ITERATION_CTX` :

```bash
TICKET_CONTEXT=$(printf '%s' "$ISSUE_JSON" | _render_ticket_context "$REPO" "$ISSUE_NUM")
if [ -n "$TICKET_CONTEXT" ]; then
    PROMPT=$(printf '%s\n\n%s' "$PROMPT" "$TICKET_CONTEXT")
fi
```

Trois invariants de position, chacun avec sa conséquence si on les viole :

1. **Après la branche `ITERATION_CTX` (`:2083-2086`).** Cette branche **réassigne** `PROMPT`
   à partir de zéro (`printf '%s#%s\n\nITERATION CONTEXT:\n%s'`) au lieu d'y appendre.
   Injecter avant ⇒ le contexte est écrasé sans bruit dès qu'une itération est en cours.
2. **Après le parse de mode à `:1801`.** Ce site teste `$PROMPT` contre un regex **ancré**
   `^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$` pour distinguer le mode repo#N du mode
   texte-libre. Injecter avant ⇒ le regex ne matche plus, dispatch bascule en texte-libre,
   **aucun worktree n'est créé**. Régression de premier ordre.
3. **Dans `_set_up_worktree`, donc avant `_detect_plan_on_branch` (`:5710`) et avant
   `_handle_dry_run` (`:5711`).** C'est ce qui donne la symétrie des deux chemins d'AC2 sans
   aucune branche conditionnelle sur `ENTRY_COMMAND` — au moment de l'injection, `ENTRY_COMMAND`
   n'est pas encore arbitré — et ce qui rend le résultat observable dans le champ `prompt` du
   JSON de dry-run (`:2110-2113`), donc AC1 et AC3 testables sans lancer de modèle.

**Duplication assumée sur le chemin B.** Sur ce chemin, `mika.md:12` re-fetche déjà le corps ;
l'injection le fournit une seconde fois. Coût borné (≤ 16 Kio), bénéfice : une seule règle —
« le pilote a toujours le texte du ticket » — sans branche conditionnelle sur un
`ENTRY_COMMAND` qui n'existe pas encore à ce point. Une branche conditionnelle achèterait
quelques kilo-octets contre une asymétrie entre les deux chemins, c'est-à-dire exactement la
classe de défaut que ce ticket ferme.

---

## 4. Contrat de vérification

Tous dans `skills/bundled/_shared/test-dispatch-lib.sh` (5 013 lignes, suite déjà gatée en CI).

**T1 — anti-vacuité, contre l'entrée réelle (AC3).** Fixture `ISSUE_JSON` portant (a) un corps
avec une consigne distinctive et (b) le commentaire d'enrichissement de mika#2158 du
`2026-09-04T09:43:28Z` (extrait littéral, ~300 o) avec une seconde consigne distinctive.
Le test **assemble l'entrée réelle du pilote** — `"$ENTRY_COMMAND $PROMPT"` avec
`ENTRY_COMMAND="/ce-work docs/plans/x.md"`, reproduisant `cli.py:290` — et asserte que
**les deux** phrases distinctives y apparaissent. Assertion sur la chaîne concaténée, jamais
sur `ISSUE_JSON` ni sur une variable interne : c'est la définition d'AC3.

**Rouge-avant, terme par terme** (`feedback_red_before_control_is_term_by_term`) — trois
contrôles négatifs **séparés**, parce que neutraliser un seul terme ne pine pas les autres :
- (a) `--json` sans `comments` → la phrase du commentaire disparaît, la phrase du corps reste ;
- (b) helper neutralisé (rend vide) → les deux phrases disparaissent ;
- (c) helper intact mais l'append supprimé → le helper rend du texte, l'entrée reste
  `\/ce-work docs/plans/x.md mika#2178`, les deux phrases disparaissent.

**T2 — symétrie des deux chemins (AC1, AC2).** Même fixture, deux assemblages : `/ce-work
docs/plans/x.md` et `/mika`. Les deux phrases distinctives apparaissent dans les deux entrées.
Le test asserte aussi que l'injection est **inconditionnelle** — aucune occurrence de
`ENTRY_COMMAND` dans le corps de `_set_up_worktree`.

**T3 — invariants de position (§3.3).** Trois assertions structurelles sur le source :
- l'append d'injection apparaît **après** la ligne `PROMPT=$(printf '%s#%s\n\nITERATION CONTEXT` ;
- la **première ligne** de `$PROMPT` reste exactement `mika#2178` — égalité stricte sur
  `head -1`, ce qui protège le contrat mika#138 et le parse ancré de `:1801` ;
- l'ordre d'appel `_set_up_worktree → _detect_plan_on_branch → _handle_dry_run` est inchangé
  (la suite porte déjà cette assertion à la ligne 260 ; T3 la référence plutôt que de la
  dupliquer).

**T4 — rendu lisible (AC1).** Fixture à deux commentaires, un de `mika-platform-dev` portant
`block[pipeline]`, un de `samidarko`. Assertions : chaque ligne d'en-tête porte le login,
l'horodatage ISO et le rôle attendu (`bot` / `humain`) ; le corps du ticket est sous son propre
séparateur `--- corps du ticket ---`.

**T5 — bornes.** (a) 15 commentaires → exactement 10 rendus, ordre chronologique croissant,
ligne d'omission portant le compte `5`. (b) un commentaire de 8 Kio → tronqué à 4 096 o avec
marqueur. (c) 10 commentaires de 4 Kio → bloc ≤ 16 384 o **et** ligne d'omission présente.
(d) un corps de 32 Kio → tronqué à 16 384 o avec marqueur.

**T6 — dégénérescences.** (a) `comments: []` → aucun séparateur de commentaire, aucune ligne
d'omission ; le corps seul est présent. (b) corps vide **et** `comments: []` → helper rend la
chaîne vide, `$PROMPT` **strictement égal** à `mika#2178` (égalité, pas `assert_contains`).

**T7 — non-régression des sept lecteurs (AC4).** Les sept sites `ISSUE_BODY`
(`:1865`, `:1873`, `:1891`, `:1902`, `:5284`, `:5294`, `:5790`) sont inchangés : assertion sur
le compte d'occurrences de `ISSUE_BODY` et sur le texte littéral de chaque ligne. Le helper
lit `ISSUE_JSON`, jamais `ISSUE_BODY` — un accès partagé rendrait les deux surfaces couplées.

**T8 — structure.** `--json state,title,labels,body,comments` présent à **un seul** site ;
`_render_ticket_context()` défini ; l'injection cite `ISSUE_JSON` et n'introduit aucun second
appel `gh`.

**T9 — hermétique.** Aucun test n'appelle `gh` : les fixtures sont du JSON littéral passé au
helper sur stdin. La suite tourne sur un runner sans réseau ni jeton.

**Commande :** `bash skills/bundled/_shared/test-dispatch-lib.sh` — toutes assertions vertes.

---

## 5. Definition of Done

- [ ] `:1836` récupère `comments` dans le **même** appel `gh issue view`.
- [ ] `_render_ticket_context()` existe, rend le corps puis les commentaires (auteur, rôle,
      date ISO par commentaire), et rend la chaîne vide quand il n'y a rien à rendre.
- [ ] Les quatre bornes (corps 16 Kio · 10 commentaires · 4 Kio par commentaire · bloc 16 Kio)
      sont appliquées et **leur raison est écrite en commentaire à côté du code**, pas
      seulement dans ce plan.
- [ ] `PROMPT` porte le bloc sur les deux chemins d'`ENTRY_COMMAND`, sans branche conditionnelle.
- [ ] Les trois invariants de position de §3.3 sont respectés et chacun porte un test (T3).
- [ ] T1–T9 passent ; T1 est rouge quand on retire le correctif, vérifié **terme par terme**
      sur trois contrôles négatifs séparés.
- [ ] `bash skills/bundled/_shared/test-dispatch-lib.sh` vert de bout en bout.
- [ ] Ticket de suivi filé pour `.claude/commands/mika.md:12` (chemin interactif manuel).
- [ ] `/ce:review` passé, TODOs résolus, `/ce:compound` produit.

## 6. Acceptance criteria

Reprise littérale des AC du corps du ticket, dans leur numérotation.

- [ ] **AC1** — Sur le chemin plan-callout (`/ce-work <plan>`), le corps **et** les
      commentaires du ticket sont injectés dans l'entrée du pilote. Mesuré sur ce que reçoit
      `run_claude_pilot` — le champ `prompt` du JSON `--dry-run` concaténé à `ENTRY_COMMAND`,
      pas sur une variable shell interne. → T1, T2, T4.
- [ ] **AC2** — Symétrie : le chemin sans callout (`/mika <repo>#N`) donne le même accès au
      corps et aux commentaires. → T2.
- [ ] **AC3 (anti-vacuité)** — Le rejeu construit l'entrée réelle
      (`/ce-work <plan> mika#N`) et vérifie que le pilote voit une consigne posée **en
      commentaire** ET une posée **dans le corps**. Un test qui remplit `ISSUE_JSON` sans
      vérifier ce que reçoit le pilote est vide. → T1 et ses trois contrôles négatifs.
- [ ] **AC4** — Non-régression : les sept lecteurs internes de `ISSUE_BODY` restent
      inchangés. → T7.

Critères de qualité ajoutés par ce plan, sans contredire les quatre ci-dessus :

- [ ] **Q1** — Bornes de volume explicites, raison de chacune écrite dans le code, toute
      omission annoncée. → T5.
- [ ] **Q2** — Un ticket sans corps ni commentaire produit un `PROMPT` strictement identique à
      aujourd'hui ; dans tous les cas la **première ligne** de `PROMPT` reste `<repo>#<num>`
      (contrat mika#138, parse ancré de `:1801`). → T3, T6.
