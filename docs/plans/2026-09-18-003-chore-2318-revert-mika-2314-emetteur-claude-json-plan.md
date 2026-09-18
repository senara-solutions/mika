---
issue: 2318
type: chore
module: dispatch-lib / claude-pilot containment
tags: [substrate, revert, prompt-cache, bwrap, surface-reduction]
problem_type: hygiene
---

# chore(2318) — reverter mika#2314, et dire ce que le revert ne prouve pas

- **Ticket :** senara-solutions/mika#2318
- **Priorité :** p2 / hygiène — non urgent (mot du ticket)
- **Branche :** `feat/2318/revert-mika-2314-metteur-claude-json`
- **Commit visé :** `88b02f97` — *« give the contained pilot a sandbox-safe ~/.claude.json so the prompt cache lives (mika#2313) (#2314) »*
- **Lignage :** mika#2108 (containment bwrap, `--tmpfs /home`), mika#2039 (aucun secret dans l'argv du sandbox), mika#2313 → PR #2316 `df13a4e3` (vraie cause : keep-alive forwardé), mika#2317 → PR #2374 `55c5fd28` (second affinage du même relais)

---

## Contexte

mika#2314 a ajouté un émetteur `~/.claude.json` sanitisé (allowlist de clés
feature-flag) et son bind `--ro-bind` dans le sandbox du pilote, sur l'hypothèse
que l'absence de ce fichier tuait le prompt-cache. L'hypothèse a été écartée : la
cause était le relais egress-proxy, corrigée depuis. Le ticket demande donc le
revert, pour réduire la surface.

**Le revert est la bonne action.** Ce plan le prescrit tel quel. Il corrige deux
choses au passage : l'argument qui le justifie (§ D1) et le sort du plan doc
(§ D2) — sans quoi le travail se ferait pour une raison qui ne tient pas, et
effacerait la seule trace de l'hypothèse écartée.

---

## Ce qui est établi, et comment le vérifier

### E1 — Le revert s'applique proprement, et le commit est en ajout pur

```
$ git show 88b02f97 | git apply --reverse --check -   # → silencieux (OK)
$ git show --stat --format="" 88b02f97
 Makefile                                    |  6 ++
 docs/plans/…-2313-…-claude-json-cache-plan.md | 58 ++++
 scripts/mika-pilot-sanitize-claude-json     | 72 ++++
 scripts/test-pilot-sanitize-claude-json.py  | 89 ++++
 skills/bundled/_shared/dispatch-lib.sh      | 30 ++
 5 files changed, 255 insertions(+)
```

**255 insertions, 0 suppression.** Rien n'a été retiré par #2314, donc rien n'est
à restaurer : le revert est un retrait net. Aucun conflit à quatre jours de
distance sur `dispatch-lib.sh`, fichier pourtant très actif.

### E2 — Aucun test existant ne dépend du bloc

`grep -n "claude.json\|CLAUDE_JSON" skills/bundled/_shared/test-dispatch-lib.sh`
→ **zéro occurrence**. Le seul test de la fonctionnalité est
`scripts/test-pilot-sanitize-claude-json.py`, qui disparaît avec elle. Le revert
ne casse aucune assertion, et n'en laisse aucune orpheline.

### E3 — Le `trap … RETURN` retiré est le seul du fichier

`grep -n "trap " dispatch-lib.sh` rend un `RETURN` unique — celui de la ligne 980,
introduit par #2314. Les autres traps sont `EXIT` (`_dispatch_lib_exit_trap`,
`kill \$_shim_pid`) et `TERM` (`_dispatch_lib_term_trap`), qu'aucune ligne du
revert ne touche. L'avertissement en tête de fichier (« Callers MUST NOT set their
own EXIT or TERM trap ») reste hors périmètre.

### E4 — La vraie cause est corrigée ET livrée, deux fois

Sur `main` : `df13a4e3` (PR #2316, *stop forwarding the client's keep-alive
upstream — the real mika#2313*), puis `55c5fd28` (`fix(2317)`, PR #2374, *le relais
ferme à la fin du CORPS, pas à l'EOF amont*). Le plan de la vraie cause
(`docs/plans/2026-09-15-002-fix-2313-egress-proxy-keepalive-plan.md`) recommande
lui-même ce revert dans sa section *Rollback* : « *Also consider reverting mika#2314
(the ~/.claude.json emitter): inert re: this bug.* »

Le mécanisme retiré ici n'est donc pas un filet dont on se priverait : le défaut
qu'il visait a un correctif nommé, mergé, et déjà itéré une fois.

### E5 — L'émetteur n'est PAS installé sur la machine de dispatch

Mesuré le 2026-09-18 sur `gentux`, le host qui dispatche :

| Sonde | Résultat |
|---|---|
| `ls ~/.local/bin/mika-pilot-sanitize-claude-json` | **`No such file or directory`** |
| `ls ~/.local/bin/mika-pilot-egress-proxy` | présent, **18 sept 12:07** |
| `ls ~/.claude.json` | présent, 37 Ko |

Les deux binaires sont posés par **la même recette** `make install` (lignes
adjacentes du `Makefile`), et le voisin y est, daté du jour. La garde du bloc est :

```bash
if [ -f "$HOME/.claude.json" ] && [ -x "$_PILOT_SANITIZE_CLAUDE_JSON_BIN" ]; then
```

Le second terme est faux. **À cet instant, sur cette machine, le bloc de #2314 est
un no-op : aucun fichier n'est généré, aucun bind n'est ajouté, le pilote tourne
cache-froid — exactement comme avant #2314.**

### E6 — Et cette inactivité ne produit aucun signal

Le seul avertissement du bloc —
`"dispatch-lib: ~/.claude.json emitter produced nothing — pilot runs cache-cold"` —
vit **à l'intérieur** du `if`, dans la branche « binaire présent mais sortie vide ».
Binaire absent ⇒ on n'entre pas dans le `if` ⇒ **silence complet**.

Un mécanisme dont on ne peut pas dire s'il est en vigueur se lit exactement comme un
mécanisme qui l'est et qui n'a rien donné. C'est la classe que mika#2205 a dû nommer
pour les scans périodiques et que mika#2293 a dû instrumenter pour les budgets LLM ;
elle se reproduit ici, sur un chemin de déploiement.

---

## Décisions

### D1 — Le revert est justifié, mais pas par l'argument que le ticket avance

Le ticket écrit : *« Le proof-groom fenêtre-claire a montré cache_read=0 sur 6 tours
ALORS QUE le fix était actif. »* La clause finale est la charnière de l'argument, et
**E5 + E6 la rendent invérifiable depuis le dépôt** : quand l'émetteur n'est pas
installé, un pilote cache-froid est précisément ce que le code produit — avec ou sans
#2314. La mesure ne sépare donc pas les deux hypothèses qu'elle prétend départager.
Le plan de #2314 avait d'ailleurs prévu ce contrôle (son AC4 : *« a dispatch without
the emitter/file stays cache_read = 0 — the mechanism is proven, not assumed »*),
sans que rien n'atteste qu'il ait été rejoué au moment du proof-groom.

**Ce n'est pas une objection au revert**, et il importe de ne pas la lire comme
telle sur un p2 d'hygiène. Trois justifications indépendantes suffisent, dont aucune
ne repose sur la clause douteuse :

1. **La vraie cause est corrigée et livrée** (E4), deux fois plutôt qu'une.
2. **Le mécanisme n'est pas déployé** (E5) : le reverter ne retire rien qui tourne.
3. **Il retire de la surface** : un bind dans le sandbox du pilote, un émetteur, et
   une dépendance d'installation qui échoue en silence (E6).

La conséquence pratique est ailleurs, et elle est écrite en § *Sonde* : le revert
**ferme la question sans y répondre**. On ne saura pas si la sanitisation aurait aidé,
puisqu'elle n'a pas tourné. Si le cache redevient froid une fois les correctifs du
relais en place, l'hypothèse `~/.claude.json` est **non-testée, pas réfutée** — et il
faut que le dépôt dise où la retrouver. C'est l'objet de D2.

### D2 — Conserver le plan doc, contre la lettre du ticket

Le ticket liste « le plan doc » parmi les artefacts à reverter. Ce plan **diverge**
et prescrit de conserver `docs/plans/2026-09-15-001-fix-2313-pilot-sandbox-claude-json-cache-plan.md`,
pour trois raisons :

1. **Le registre des plans est append-only, sans exception.** 878 fichiers sous
   `docs/plans/` ; `git log --diff-filter=D -- docs/plans/` rend **zéro**
   suppression sur toute l'histoire du dépôt. Ce serait la première.
2. **Le plan de la vraie cause le référence.** `2026-09-15-002` s'ouvre sur *« The
   earlier hypothesis (missing ~/.claude.json) was refuted »* ; supprimer `…-001`
   laisse cette phrase sans son objet.
3. **Supprimer est ce qui fera re-proposer l'hypothèse.** C'est le seul endroit qui
   documente ce qui a été essayé et sur quelles mesures ; et vu D1, l'hypothèse
   n'est pas close.

Contrepartie, nommée : un plan conservé qui décrit un correctif retiré ment par
omission pour qui le lit seul. D'où l'encadré de réfutation en tête (§ Changements 5),
qui est la moitié non négociable de cette décision.

*Si l'architecte tranche pour la lettre du ticket :* supprimer le fichier et porter
l'encadré dans `2026-09-15-002`, faute de quoi la référence de la raison 2 devient
orpheline.

### D3 — Le déploiement exige rebuild + restart, pas un `cp` — vérifié en source

Le NB du ticket est exact, et voici par quoi :
`crates/mika-agent/build.rs:205` appelle `discover_support_dirs()` sur
`skills/bundled/`, qui embarque `_shared/dispatch-lib.sh` dans le binaire ;
`startup::seed_bundled_skills_if_needed` (`startup.rs:80`) appelle
`bundled_skills::seed_support_dirs`, dont le corps (`bundled_skills.rs:1009`) écrit
`write_dir_files(&target_dir, dir.files)` **sans condition d'existence** — donc
réécrit le `dispatch-lib.sh` installé **à chaque démarrage**, depuis le contenu
compilé.

Un `cp` manuel du fichier reverté serait donc écrasé au restart suivant : le revert
n'est effectif qu'après `make build` (ou `make deploy`) **et** redémarrage de
mika-spirit.

### D4 — Ne pas coder la désinstallation de l'émetteur

E5 dit qu'il n'est pas là. Ajouter un pas de suppression au `Makefile` serait du
code pour un état qui n'existe pas, dans une cible `install` qui n'a pas de pendant
`uninstall`. S'il réapparaît sur une machine (un `make install` joué depuis un
checkout pré-revert), il devient un fichier orphelin inerte : après ce revert, plus
aucun appelant ne le nomme. Geste opérateur d'une ligne, noté en § *Déploiement*,
pas automatisé.

---

## Changements

Le revert est mécanique (E1) : `git revert --no-commit 88b02f97`, moins le plan doc
(D2), plus l'encadré. Détail, pour que la revue puisse pointer :

### 1. `scripts/mika-pilot-sanitize-claude-json` — supprimé

### 2. `scripts/test-pilot-sanitize-claude-json.py` — supprimé

### 3. `Makefile` — retirer les 6 lignes d'install (~50-55)

Le commentaire `@# mika#2313: sandbox-safe ~/.claude.json emitter` et son bloc
`cp` / `chmod` / `mv` / `echo`. La recette voisine
(`mika-pilot-egress-proxy`, **à conserver** — c'est le correctif de la vraie cause)
et celle de l'addon auth restent intactes.

### 4. `skills/bundled/_shared/dispatch-lib.sh` — quatre retraits

| Lignes | Contenu |
|---|---|
| ~289-296 | commentaire de 7 lignes + `_PILOT_SANITIZE_CLAUDE_JSON_BIN=` |
| ~970-987 | bloc de génération dans `_run_pilot_sandboxed` : commentaire, `local -a _PILOT_CLAUDE_JSON_BIND_ARGS`, `local _PILOT_CLAUDE_JSON`, `mktemp`, `trap … RETURN` (E3), l'appel de l'émetteur, la branche `else` et son `echo` |
| **1197** | `${_PILOT_CLAUDE_JSON_BIND_ARGS[@]+"…"} \` — 1ʳᵉ invocation `bwrap` |
| **1282** | idem — 2ᵈᵉ invocation `bwrap` |

*Point d'attention à l'implémentation :* les deux dernières lignes vivent au milieu
d'une commande `bwrap` continuée par `\`. Retirer la ligne entière, jamais seulement
son contenu — une ligne vide entre deux `\` de continuation casse la commande, et
`bash -n` **ne l'attrape pas** (la syntaxe reste valide, c'est l'invocation qui perd
ses arguments suivants). C'est le seul risque non trivial du revert, et il est
couvert par V5.

### 5. `docs/plans/2026-09-15-001-…-claude-json-cache-plan.md` — **conservé**, encadré ajouté

Un bloc en tête (après le frontmatter, avant le `# fix(2313)`), qui doit dire trois
choses et pas une de plus : que le correctif décrit **a été reverté** par mika#2318 ;
que la vraie cause était le relais (#2313 → PR #2316, puis #2317) ; et — la phrase
qui compte — que l'hypothèse `~/.claude.json` est **non-testée plutôt que réfutée**,
l'émetteur n'ayant pas été trouvé installé sur la machine de dispatch (E5). Avec le
SHA `88b02f97` nommé, pour que le code retiré reste retrouvable.

---

## Vérification

| # | Contrôle | Attendu |
|---|---|---|
| V1 | `git show 88b02f97 \| git apply --reverse --check -` | silencieux (déjà vert, E1) |
| V2 | `grep -rn "sanitize-claude-json\|_PILOT_CLAUDE_JSON\|_PILOT_SANITIZE" --include='*.sh' --include='*.rs' --include='Makefile' .` | **0 occurrence** hors `docs/plans/` |
| V3 | `bash -n skills/bundled/_shared/dispatch-lib.sh` | OK |
| V4 | `skills/bundled/_shared/test-dispatch-lib.sh` | 616 passés. **Attention :** 88b02f97 documente **1 échec pré-existant** sans rapport, vérifié par stash à l'époque — le constater et ne pas le lire comme une régression du revert |
| V5 | **Un dispatch sandboxé réel qui aboutit** | le pilote démarre et livre. C'est le seul contrôle qui couvre le retrait des deux expansions au milieu des `bwrap` (§ Changements 4) ; ni `bash -n` ni la suite shell ne le font |
| V6 | `make verify-bundled-skills` | passe |
| V7 | `cargo build` | passe — `dispatch-lib.sh` est embarqué (D3), donc sa modification est un input de build |

---

## Déploiement

1. `make build` (ou `make deploy`) — le contenu embarqué change (D3).
2. **Redémarrer mika-spirit.** `seed_support_dirs` réécrit alors le
   `dispatch-lib.sh` installé depuis le binaire neuf. Sans ce restart, le revert
   n'est pas en vigueur, quel que soit l'état du dépôt.
3. Facultatif, sur toute machine où le fichier existe :
   `rm -f ~/.local/bin/mika-pilot-sanitize-claude-json` (D4). Il n'est pas sur la
   machine de dispatch (E5).

---

## Sonde post-déploiement, et sa halte

Sur le premier dispatch sandboxé après restart : **`cache_read > 0` au 2ᵉ tour du
transcript et tours < 60 s** — c'est-à-dire l'AC3 de #2313, qui doit tenir **avant
comme après** ce revert, puisque le mécanisme retiré n'était pas en vigueur (E5).

- **Cache chaud, tours courts** → attendu. Le revert n'a rien coûté, et E5 est
  confirmé par le comportement.
- **Cache redevenu froid après le revert** → **halte.** Ne pas re-committer
  `88b02f97` par réflexe : ce résultat dirait que le mécanisme *était* en vigueur
  quelque part, donc que E5 ne vaut pas pour la machine qui a réellement dispatché —
  et il contredirait du même coup le diagnostic de #2313. Établir d'abord **sur quelle
  machine** l'émetteur est installé (`ls ~/.local/bin/mika-pilot-sanitize-claude-json`
  sur chaque host de dispatch), avant toute conclusion.

C'est ce qui rend ce revert falsifiable, et c'est la contrepartie de D1 : on ne
prétend pas avoir réfuté l'hypothèse, on prétend qu'elle ne coûte rien à retirer et
on dit à quoi ressemblerait le contraire.

---

## Hors périmètre, délibérément

- **Rouvrir le diagnostic du prompt-cache.** #2313/PR #2316 et #2317/PR #2374 ont
  traité le relais ; ce travail retire une piste écartée, il ne re-tranche rien.
- **Instrumenter l'absence de l'émetteur** (le trou d'observabilité de E6) —
  ce serait instrumenter du code qu'on supprime. E6 est un argument *pour* le
  revert, pas un défaut à corriger dedans. La classe générale (un chemin de
  déploiement dont l'inactivité est muette) mérite son propre ticket si elle se
  reproduit ailleurs.
- **Une cible `make uninstall`** (D4).
- **L'échec pré-existant de `test-dispatch-lib.sh`** (V4) : antérieur à #2314,
  sans rapport.
- **`~/.claude.json` du host lui-même** : ce revert ne touche que le sandbox du
  pilote ; le fichier de l'opérateur est hors sujet.

---

## Definition of Done

1. Les deux scripts sont supprimés, les 6 lignes du `Makefile` et les quatre
   retraits de `dispatch-lib.sh` sont faits, `git diff main` ne montre que ça plus
   l'encadré.
2. `docs/plans/2026-09-15-001-…-plan.md` existe toujours et porte l'encadré de
   réfutation nommant `88b02f97`, mika#2318 et la nuance « non-testée » (D2).
3. V1 à V7 passent, **V5 incluse** (un dispatch sandboxé réel qui aboutit).
4. Le corps de PR nomme le geste de déploiement (rebuild + restart, D3) — le revert
   est inerte sans lui.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria`. Les critères
ci-dessous sont dérivés de sa section « À reverter » et des constats E1-E6.

- **AC1** — Les cinq artefacts de `88b02f97` listés par le ticket sont traités :
  les deux scripts supprimés, l'install `Makefile` retirée, le bloc
  génération/bind retiré de `dispatch-lib.sh`. Le cinquième (plan doc) est
  **conservé avec encadré** plutôt que supprimé — divergence motivée par D2, à
  trancher en revue.
- **AC2** — Aucune référence résiduelle : `grep` sur `sanitize-claude-json`,
  `_PILOT_CLAUDE_JSON` et `_PILOT_SANITIZE` rend zéro occurrence hors
  `docs/plans/` (V2).
- **AC3** — Le pilote sandboxé **démarre et livre** après le revert : un dispatch
  réel aboutit (V5). C'est l'AC qui couvre le retrait des deux expansions de
  tableau au milieu des invocations `bwrap`, qu'aucune analyse statique n'attrape.
- **AC4** — Aucune régression de la suite `dispatch-lib` : 616 tests passés, et
  l'échec pré-existant documenté par `88b02f97` est identifié comme tel, pas
  compté comme une régression (V4).
- **AC5** — Le trou d'observabilité est consigné : le plan (et l'encadré de D2)
  énoncent que l'émetteur n'a pas été trouvé installé sur la machine de dispatch
  (E5) et que le bloc ne signalait pas son inactivité (E6) — donc que l'hypothèse
  `~/.claude.json` est retirée **sans avoir été testée**, et non réfutée.
- **AC6** — Le geste de déploiement est nommé dans le corps de PR : rebuild
  mika-spirit **et** restart, `cp` inopérant car `seed_support_dirs` réécrit le
  fichier installé à chaque démarrage (D3).
- **AC7** — La sonde post-déploiement et sa halte sont exécutables telles
  qu'écrites : un dispatch après restart doit montrer `cache_read > 0` au 2ᵉ tour,
  et un cache froid **arrête** le travail au lieu de déclencher un re-commit.
