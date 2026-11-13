# Vol 3 de mika#2077 — un pilote de revise tué n'est pas un modèle qui a refusé

> Ticket : senara-solutions/mika#2077 (parapluie p3 du substrat de dispatch).
> Ligne traitée : « `_launch_revise_pilot` est hors du chemin de terminaison » (reprise de #2033).
> Vol 1 : `docs/plans/2026-10-19-005-fix-2077-entrypoint-phase-2b-argv-positionnel-plan.md`, livré par #2827.
> Vol 2 : `docs/plans/2026-10-19-006-fix-2077-demotion-du-marqueur-pipeline-failure-plan.md`, livré par #2831.

## État des cinq lignes du parapluie, à la date de ce plan

| ligne | source | état pour ce vol |
|---|---|---|
| Six suites de `_shared/tests/` ne s'exécutent pas | #2058 | **hors périmètre** — recoupe senara-solutions/mika#2826, re-mesurée `OPEN` / `mergedAt: null` le 2026-10-19. Le corps du parapluie le dit : « sans être refaites ici ». |
| L'entrypoint Phase 2b re-cite l'argv avec `printf '%q'` | #2057 | **cochée** — vol 1, #2827 |
| `_launch_revise_pilot` est hors du chemin de terminaison | #2033 | **ce vol** |
| `sed` retire une ligne d'un `PIPELINE FAILURE` multi-ligne | #2032 | **cochée** — vol 2, #2831 |
| `test_sandbox_log_dir_bound.sh` échoue en rouge là où elle devrait sauter | vol #2827 | **hors périmètre** — même recoupement avec #2826 |

Ce vol coche **une** ligne. Les deux lignes qui recoupent #2826 restent décochées : leur preuve
est le merge de #2826, pas un commit d'ici.

**État de la branche, mesuré.** Les deux commentaires d'auto-pull du ticket refusent la
promotion pour « 4 commits de retard, 5 fichiers hors `docs/plans/` ». Ils sont **périmés** :
`git log origin/main..HEAD` et `git log HEAD..origin/main` sont tous deux vides, et
`git diff origin/main --stat` ne rend rien — la branche est strictement identique à
`origin/main`. Le travail partiel que la porte voyait est précisément ce qui a été mergé par
#2827 puis #2831. Il n'y a donc **rien à sauver ni à abandonner** : ce vol part d'une branche
propre, et le `operator-gated` posé par la porte n'a plus de sujet.

## Le défaut

### Ce que la ligne du parapluie dit, et ce que #2033 dit

Le corps du parapluie glose la ligne en « un lancement qui échappe au nettoyage laisse un
processus orphelin ». **Cette glose est fausse**, et elle n'est pas la ligne : le titre de
#2033 est *« `_launch_revise_pilot` sits outside the terminated-session honesty guarantee »*,
et son corps ne parle à aucun moment de processus orphelin ni de nettoyage. Il n'y a pas de
fuite de processus à fermer ici — les deux `mktemp` sont bien supprimés (`:9266`), et
`_run_pilot_sandboxed` porte son propre `trap ... EXIT` pour le shim d'egress (`:2532`), un
trap que `:2117-2128` documente comme délibérément non dispatch-level *parce que*
`_run_pilot_sandboxed` tourne plus d'une fois par dispatch — ce lancement-ci compris.

Le périmètre de ce vol est donc celui de #2033 et du § *Découpage en vols* du vol 2 :
**la garantie d'honnêteté de terminaison**, qui s'arrête à un site d'appel du pilote.

### Le site

`dispatch-lib.sh:9249-9266`. `_launch_revise_pilot` lance le pilote de revise, capture sa
sortie dans deux `mktemp`, et les supprime **sans jamais les lire** :

```bash
_run_pilot_sandboxed claude-pilot --verbose … --command "/mika-revise-plan" … \
    >"$revise_stdout" 2>"$revise_stderr"
local revise_exit=$?
…
local post_hash; post_hash=$(sha256sum "$plan_path" | cut -d' ' -f1)
rm -f "$revise_stdout" "$revise_stderr"
```

Le seul prédicat de convergence est le sha256 du plan. Quand il n'a pas bougé, la fonction
rend un unique message (`:9281`) :

```
WARN: _launch_revise_pilot: plan unchanged after revise pilot (exit=$revise_exit)
```

### Trois populations, une seule phrase

Ce message est la seule chose que la boucle apprend, et il confond **trois** causes que le
substrat sait déjà distinguer partout ailleurs :

1. **Le modèle a lu les findings et n'a rien fait.** `status: success`, session conclue, plan
   intact. C'est la seule population pour laquelle « plan unchanged » est vrai.
2. **La session a été tuée avant d'arriver au plan.** `status: terminated`, `.subtype` =
   `idle_timeout`, `error_max_turns`, `rate_limited`… PR#2028 a rendu cette population
   mesurable *pour le pilote principal* (`:5479-5510`, puis `_classify_terminated_session` à
   `:6274`), et R5 du plan de mika#1772 avait explicitement borné le correctif au pilote
   principal en consignant le reste comme trou. Ici, rien n'est extrait : un `idle_timeout`
   est rapporté comme un modèle qui a refusé de réviser.
3. **Le pilote n'a jamais existé.** `_run_pilot_sandboxed` rend **78** quand il refuse de
   construire le sandbox (`:2588`, `:2598`, `:2648`), en posant le motif dans
   `$_PILOT_SANDBOX_REFUSAL` (`:2576`). Le chemin principal traite ce cas à part et dit
   pourquoi — « this is a refusal, not drift and not a pipeline failure » (`:5537-5562`).
   Le chemin de revise le rend en `exit=78` nu, à l'intérieur d'une phrase qui parle du plan.

### La seconde moitié : « no reason recorded »

Le site appelant (`:10287`) :

```bash
_launch_revise_pilot "$findings_file" || {
    _groom_warn "revise pilot did not converge — preserving $findings_file for forensics"
    return 1
}
```

`_groom_warn` (`:9929`) est bien appelé, donc `GROOM_LOOP_FAILURE_REASON` **est** posé — la
contrainte (ii) du vol 2 est ici à corriger d'un cran : le terminal ne dit pas
« no reason recorded » (`:12568`), il dit *« revise pilot did not converge »*. Le défaut est
plus discret et pire : la phrase est **une affirmation de contenu**, et elle est fausse sur
deux des trois populations. Un opérateur qui la lit ouvre les findings de l'architecte et le
plan ; la cause est en amont du pilote.

« did not converge » est en outre le vocabulaire que `:12602` réemploie pour l'enveloppe
(`PIPELINE FAILURE: grooming did not converge — …`), ce qui rend la phrase doublement
plausible et donc doublement coûteuse à mettre en doute.

## Les deux contraintes mesurées, et ce qu'elles imposent

### (i) Les globales de session ne peuvent pas être écrasées

`_classify_terminated_session` (`:6274`) et `_resolve_halt_subtype` (`:6232`) ne prennent pas
d'argument : elles **lisent des globales** — `SUBTYPE`, `TERMINATION_REASON`,
`API_ERROR_STATUS`, `SESSION_ID`, `TURNS`, `DURATION`, `STDERR_FILE`, `LOG_ID` (`:6086-6087`,
`:6229`). Ces globales portent la session du **pilote principal**, et elles ont des lecteurs
**en aval** de `_iterate_groom_loop` (`:12505`) :

- `_signal_rescue_into_open_pr`, appelée à `:12631`, écrit `- Session pilote : <SESSION_ID>`
  dans un commentaire de PR (`:8208`). Un `SESSION_ID` écrasé par le pilote de revise ferait
  poster, sur la PR, la session **du revise** comme provenance des commits du filet.
- `_escalate_groom` (`:9598`), appelée à `:10377` et `:10382` — c'est-à-dire **après**
  `_launch_revise_pilot` sur les deux sorties ESCALATE du même bras.

Donc : classification de la session de revise **sans aucune écriture** dans ces globales.
La forme qui le garantit par construction est la **substitution de commande** — déjà un
sous-shell — avec les liaisons posées *dedans*.

### (i-bis) Le mémo de halte s'hérite, et c'est le piège inverse

`_resolve_halt_subtype` est **mémoïsé** et son garde est `[ -n "${_HALT_SUBTYPE_RESOLVED+x}" ]`
(`:6236`) — une résolution vide est une résolution. Le mémo est posé par le post-flight du
pilote principal (`:5503` le remet à zéro, `:5598` le résout). Un sous-shell **hérite** de ce
mémo : un appel naïf rendrait donc la halte du **pilote principal** comme halte du pilote de
revise. C'est une fausse attribution *pire* que celle d'aujourd'hui — elle est précise, donc
crédible. Le `unset` à l'intérieur du sous-shell n'est pas une hygiène, c'est la correction.

Pour la même raison, `STDERR_FILE` et `LOG_ID` doivent être reliés aux fichiers du revise :
le repli par scrape de la ligne `[guardrail]` (`:6256-6262`) lirait sinon le stderr du pilote
principal.

### (ii) Le diagnostic doit tenir sur UNE ligne, et sans le marqueur

`GROOM_LOOP_FAILURE_REASON` est interpolé dans un **remplacement `sed`** à `:12595` :

```bash
RESULT=$(printf '%s' "$RESULT" | sed "s/Outcome: .*/Outcome: PIPELINE_INCOMPLETE — ${_groom_reason_sed}/")
```

L'échappement de `:12571-12573` couvre `\`, `/` et `&` — **pas** le saut de ligne. Un motif
multi-ligne donne `unterminated 's' command`, `sed` n'écrit rien, et la substitution de
commande rend une chaîne vide : **tout le corps du `RESULT` est perdu**, pilote compris. La
propriété « une seule ligne » est donc une exigence de *correction*, pas de style.

Conséquence de conception : on ne peut **pas** réutiliser la sortie de
`_classify_terminated_session`, qui est un bloc multi-ligne ouvert par `PIPELINE FAILURE:`.
#2033 suggérait cette réutilisation (« already a standalone, callable function ») ; elle est
mesurée inadaptée à *ce* consommateur. Ce vol réutilise la **classification** —
`_resolve_halt_subtype` et `_halt_family` (`:6116`), les deux sites de vérité que
`_classify_terminated_session` lit elle-même — et non la **présentation**. Aucun vocabulaire
de halte n'est donc recopié : le `case` de `:6116` reste l'unique table, et le garde de dérive
de `test-dispatch-lib.sh` qui lit le `Literal` amont continue de la couvrir seule.

Le diagnostic ne doit pas non plus porter la sous-chaîne `PIPELINE FAILURE:` :
`evaluator.rs:305` la lit en `contains`, non ancré — la forme que le vol 2 vient de fermer.
En revanche `Halt class:` est lu **ancré** (`line_value`, `evaluator.rs:357-363`, `starts_with`
par ligne), donc une mention de la classe en milieu de ligne est inerte pour lui ; le
diagnostic la nomme quand même en minuscules, pour qu'aucun futur lecteur non ancré n'ait de
prise.

## Requirements

- **R1.** `_launch_revise_pilot` extrait de `$revise_stdout`, **avant** le `rm -f`, le premier
  objet JSON de la sortie (`grep -m1 '^{'`, la forme du chemin principal à `:5474`) et en lit
  `status`, `subtype`, `termination_reason`, `api_error_status`, `session_id`, `turns`,
  `duration_ms` dans des variables **locales**.
- **R2.** Une fonction `_classify_revise_halt` compose **une seule ligne** nommant la halte de
  la session de revise : sous-type, détail (`termination_reason`), classe et indice issus de
  `_halt_family` / `_halt_hint_meaning`. Elle est appelée en substitution de commande, et elle
  `unset` `_HALT_SUBTYPE_RESOLVED` / `_HALT_GUARDRAIL_LINE` et relie `SUBTYPE`,
  `TERMINATION_REASON`, `API_ERROR_STATUS`, `STDERR_FILE`, `LOG_ID` **à l'intérieur** du
  sous-shell. Aucune globale de session n'est écrite par ce chemin.
- **R3.** Sur la branche « plan inchangé », le message et le diagnostic publié distinguent les
  trois populations du § ci-dessus :
  - `revise_exit = 78` → refus de containment, motif pris dans `$_PILOT_SANDBOX_REFUSAL`, et
    la phrase dit que **le pilote n'a jamais été lancé** ;
  - `status = terminated` → échec **de session**, avec la ligne de R2, et la phrase dit que le
    plan n'a pas été révisé parce que la session a été tuée, non parce que le modèle a refusé ;
  - sinon → la phrase actuelle, qui devient enfin vraie : la session a conclu et le plan est
    resté identique.
- **R4.** `_launch_revise_pilot` publie le diagnostic dans une globale
  `REVISE_PILOT_HALT_DIAGNOSIS`, remise à zéro **à chaque entrée** (même doctrine que
  `_FD_REVISE_RETRIED` à `:9220`), et posée sur **chacun** de ses `return 1` — les quatre
  gardes d'entrée (`:9231`, `:9234`, `:9236`, `:9242`) comprises, qui aujourd'hui sortent sans
  rien publier.
- **R5.** Le diagnostic est **une seule ligne** et ne contient pas la sous-chaîne
  `PIPELINE FAILURE:`.
- **R6.** Le site appelant `:10287` reporte le diagnostic dans `_groom_warn`, à la place de la
  phrase de contenu actuelle. `_groom_warn` reste l'unique enregistreur de
  `GROOM_LOOP_FAILURE_REASON` — « recording and warning in one act », `:9924-9928` ;
  `_launch_revise_pilot` publie un *diagnostic*, pas un motif d'échec de boucle.
- **R7.** Une session de revise **terminée qui a quand même changé le plan** (l'analogue du
  `_terminated_with_work` principal, `:5613-5622`) émet la ligne de R2 sur stderr et **ne
  change rien d'autre** : `return 0`, la boucle enchaîne sur la seconde passe. Le plan a été
  écrit par une session incomplète, et c'est un fait à dire, pas une porte à fermer.
- **R8.** Un détecteur mesure R1–R7, y compris les deux sens de la contrainte (i) — aucune
  fuite vers les globales, aucune fuite *depuis* le mémo — et porte une partie anti-vacuité
  sur copies régressées.
- **R9.** Le détecteur est atteint par le pipeline **sans nouveau câblage** : il atterrit dans
  `skills/bundled/_shared/test-dispatch-lib.sh`, que `make test-dispatch-lib` exécute
  (`Makefile:199-200`) et que le job `Check` lance à `ci.yml:85`. C'est aussi le fichier où
  vivent déjà les deux autres sondes comportementales de cette même fonction
  (`_t2306_revise_probe` à `:9068`, `_t2636_revise_probe` à `:12091`), dont la sonde de ce vol
  réemploie la technique de neutralisation.

## Non-requirements

- **Aucun nettoyage de processus.** La glose « processus orphelin » du corps du parapluie est
  mesurée fausse (voir § *Ce que la ligne du parapluie dit*). Rien à fermer.
- **Pas de réutilisation de la sortie de `_classify_terminated_session`.** Multi-ligne et
  ouverte par `PIPELINE FAILURE:` : incompatible avec le `sed` de `:12595`. La classification
  est réutilisée, la présentation non.
- **Pas de factorisation du `grep -m1 '^{'`.** Elle toucherait le post-flight du pilote
  principal, c'est-à-dire la seule garantie d'honnêteté que ce vol ne doit pas déranger. La
  duplication est **épinglée** à la place : une assertion exige que les deux sites portent la
  forme identique, donc une dérive rougit au lieu de vieillir.
- **Pas de nouveau comportement de retry.** Les trois populations deviennent lisibles ; rien
  ne décide d'un re-dispatch à partir d'elles. Les indices de `_halt_family` sont des
  annotations, et `:6096-6097` dit qu'elles le restent.
- **Pas de seconde itération de revise.** Le budget d'itération de la boucle est inchangé ;
  les deux rattrapages bornés existants (`:9270`, `:9278`) ne sont pas touchés.
- **Pas de changement du prédicat de convergence.** Le sha256 reste la mesure du « plan
  révisé ». Ce vol ajoute une mesure de la *session*, orthogonale.

## Verification contract

Les V1–V9 passent par une **sonde comportementale** calquée sur `_t2306_revise_probe`
(`test-dispatch-lib.sh:9068-9147`) : la vraie `_launch_revise_pilot` s'exécute, seul
`_run_pilot_sandboxed` est redéfini dans le sous-shell. Le stub écrit son JSON sur **stdout**
— que `:9260` redirige déjà vers `$revise_stdout` — et sa ligne `[guardrail]` sur stderr, donc
c'est la fonction réelle qui capture, lit et classe. Le plan n'est **pas** muté, pour que
`pre_hash == post_hash` et que la branche d'échec soit atteinte.

| # | ce qui est mesuré | comment |
|---|---|---|
| V1 | une session de revise `terminated` est rapportée comme un échec **de session** | stub rendant `status: terminated`, `subtype: idle_timeout` ; le diagnostic porte `idle_timeout` et `session_silent` |
| V2 | …et ne se présente plus comme un refus du modèle | le diagnostic ne contient pas `plan unchanged` ; il dit que la session a été tuée |
| V3 | une session conclue sans révision garde la phrase de contenu | stub rendant `status: success` ; le diagnostic nomme le plan, pas de halte |
| V4 | un `exit 78` est rapporté comme un pilote **jamais lancé** | stub posant `_PILOT_SANDBOX_REFUSAL` et rendant 78 ; le diagnostic porte le motif et pas de classe de halte |
| V5 | **aucune fuite vers les globales** (contrainte i) | `SESSION_ID`/`SUBTYPE`/`STATUS`/`TURNS`/`DURATION`/`TERMINATION_REASON` pré-posés aux valeurs du pilote principal ; octet pour octet inchangés après l'appel |
| V6 | **aucune fuite depuis le mémo** (contrainte i-bis) | `_HALT_SUBTYPE_RESOLVED=error_max_turns` pré-posé ; le diagnostic dit `idle_timeout`/`session_silent`, jamais `error_max_turns`/`budget_exhausted` |
| V7 | le diagnostic est **une seule ligne** et sans le marqueur | `wc -l` → 1 sur les quatre populations ; `grep -cF 'PIPELINE FAILURE:'` → 0 |
| V8 | les quatre gardes d'entrée publient un diagnostic | les quatre `return 1` de `:9231-9242` rejoués ; `REVISE_PILOT_HALT_DIAGNOSIS` non vide et nommant la garde ; et la globale est remise à zéro à chaque entrée |
| V9 | le site appelant reporte le diagnostic | scan du bras `ITERATE` : `_groom_warn` y lit `REVISE_PILOT_HALT_DIAGNOSIS`, et la phrase de contenu nue a disparu |
| V10 | un `terminated` **avec** plan changé dit la halte sans rien bloquer | stub `terminated` qui mute le plan ; `rc = 0`, et stderr porte la ligne de halte |
| V11 | la table de haltes n'est pas recopiée | `_classify_revise_halt` ne contient aucun sous-type littéral : son `case` est celui de `_halt_family`, lu par appel |
| V12 | la forme d'extraction JSON est identique aux deux sites | `grep -m1 '^{'` présent dans `_launch_revise_pilot` et dans le post-flight principal, forme identique |
| V13 | anti-vacuité, copie A | copie de `dispatch-lib.sh` dont le `unset` du mémo est retiré → **V6 doit échouer**, et elle seule (compte d'échecs exact) |
| V14 | anti-vacuité, copie B | copie restaurant le message `plan unchanged after revise pilot` d'origine → **V1, V2, V4 et V9 doivent échouer**, avec le compte exact |
| V15 | préconditions | ni bwrap, ni réseau, ni token : la sonde ne lance aucun pilote réel |

V13 et V14 sont **deux** copies parce qu'elles ne peuvent pas échouer sur le même défaut :
retirer le `unset` laisse la classification en place (seule V6 bascule), et restaurer l'ancien
message laisse le mémo correct (V6 reste verte). Une seule copie régressée donnerait un vert
sur la moitié non couverte — la leçon que le vol 2 a dû écrire sur son propre `sed`.

## Definition of Done

- `_classify_revise_halt` existe, contenue par substitution de commande, mémo `unset` dedans.
- `_launch_revise_pilot` lit la sortie du pilote de revise avant de la supprimer, distingue les
  trois populations, et publie `REVISE_PILOT_HALT_DIAGNOSIS` sur chacun de ses `return 1`.
- Le site `:10287` reporte le diagnostic ; la phrase de contenu nue a disparu.
- V1–V15 vertes dans `skills/bundled/_shared/test-dispatch-lib.sh`.
- `bash skills/bundled/_shared/test-dispatch-lib.sh` et les suites voisines du substrat
  (`make test-dispatch-lib`) restent vertes — T2306 et S2636 comprises, qui exercent la même
  fonction.
- `make check-suite-wiring` passe **sans nouvelle entrée** dans `scripts/suite-wiring.tsv` :
  aucun fichier de suite n'est créé.
- `scripts/verify-pipeline.sh origin/main` passe.
- Le corps de mika#2077 coche la ligne `_launch_revise_pilot` avec la PR de ce vol pour preuve,
  et **laisse décochées** les deux lignes qui recoupent #2826.

## Acceptance criteria

Le ticket mika#2077 n'a pas de section `## Acceptance criteria` ; les critères ci-dessous sont
dérivés des Requirements et du Verification contract.

- **AC1.** Un pilote de revise tué par un garde-fou produit un rapport qui nomme la halte —
  sous-type, classe, indice — et qui dit explicitement que le plan n'a pas été révisé parce que
  la session a été tuée. Mesuré par V1 et V2.
- **AC2.** Un pilote de revise qui a conclu sans toucher au plan garde le rapport de contenu, et
  c'est le seul cas où il est rendu. Mesuré par V3.
- **AC3.** Un refus de containment (`exit 78`) est rapporté comme un pilote jamais lancé, avec
  le motif du refus. Mesuré par V4.
- **AC4.** La classification de la session de revise n'écrit **aucune** globale de session :
  le commentaire de `_signal_rescue_into_open_pr` (`:12631`) et les deux `_escalate_groom`
  (`:10377`, `:10382`) continuent de nommer la session du pilote principal. Mesuré par V5.
- **AC5.** La classification de la session de revise ne rend **jamais** la halte du pilote
  principal : le mémo hérité est neutralisé. Mesuré par V6.
- **AC6.** Le diagnostic reporté dans `GROOM_LOOP_FAILURE_REASON` tient sur une ligne et ne
  porte pas `PIPELINE FAILURE:`, donc le `sed` de `:12595` ne peut pas perdre le corps du
  `RESULT`. Mesuré par V7.
- **AC7.** Aucune sortie en échec de `_launch_revise_pilot` ne laisse le terminal sans motif,
  gardes d'entrée comprises, et le site appelant reporte bien ce motif. Mesuré par V8 et V9.
- **AC8.** Une session terminée qui a néanmoins révisé le plan dit sa halte et ne bloque rien.
  Mesuré par V10.
- **AC9.** Le vocabulaire de halte garde un site de vérité unique, et la forme d'extraction
  JSON ne peut pas dériver entre les deux sites en silence. Mesuré par V11 et V12.
- **AC10.** Le détecteur sait dire non, séparément sur ses deux défauts. Mesuré par V13 et V14.
- **AC11.** Le détecteur est exécuté par le job `Check` de toute PR, sans aucun câblage neuf et
  sans exception déclarée. Mesuré par `ci.yml:85` et `make check-suite-wiring`.

## Fire-Disposition

Ce plan livre un détecteur : les V1–V15 ajoutées à `test-dispatch-lib.sh`, dont le chemin de
succès est « aucune population confondue, aucune globale touchée ».

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est vide.**

- **L'ensemble des exceptions est vide par construction, pas par omission.** Le détecteur et le
  correctif atterrissent dans le même commit, et le détecteur ne mesure que la fonction neuve,
  les quatre arms de `_launch_revise_pilot` et son unique site d'appel
  (`grep -c '_launch_revise_pilot' dispatch-lib.sh` hors commentaires → une définition, un
  appel). Il n'existe aucune violation préexistante à excepter : le site `:9281` est le seul, et
  il est réparé dans ce vol. Le détecteur atterrit donc **armé et vert**, et non désarmé
  (option b).
- **Ce qui empêche « vide » de se lire comme « vacant ».** V13 et V14 : deux copies régressées
  de `dispatch-lib.sh`, écrites sous `.pilot-scratch/`, avec un **compte d'échecs exact** par
  copie. Une seule copie ne suffit pas, et le § *Verification contract* dit pourquoi : les deux
  défauts de ce vol sont indépendants, et chaque copie n'en rejoue qu'un.
- **Ce qui ferait rougir l'exception si elle naissait.** Un second lancement de pilote hors du
  chemin de classification n'a pas d'entrée d'allowlist à prendre : V12 épingle la forme
  d'extraction, V11 épingle la table de haltes, et V5 épingle l'absence d'écriture dans les
  globales — trois assertions qui sont des propriétés du fichier, pas des listes à tenir à
  jour. L'auto-nettoyage est donc structurel : il n'y a aucune entrée à devenir stale.
- Option (c) écartée : rien ici ne demande de cadrage opérateur. Le périmètre est le résidu
  d'un ticket déjà tranché (#2033), et la branche est propre et à jour.

## Découpage en vols

Le parapluie avait deux lignes traitables ; le vol 2 a pris la première (`sed`) et a renvoyé
celle-ci au vol 3, en nommant ses deux contraintes mesurées. **Ce vol est le dernier des trois
sur les lignes traitables** : après lui, les deux lignes restantes du parapluie sont des
recoupements de #2826, cochables à son merge et par rien d'ici.

Un seul vol, et il tient (263 < 279) parce que le correctif est une fonction d'une quinzaine
de lignes utiles plus trois arms dans une fonction existante, sur un site d'appel unique et
déjà localisé ; le volume est aux deux tiers dans le détecteur et dans la doctrine, qui ne
demandent ni exploration ni itération. Aucun code Rust, donc aucun cycle `cargo build` : les
deux lectures Rust de ce plan (`evaluator.rs:305` en `contains`, `line_value:357-363` en
ancré) sont des **mesures** déjà faites, pas des fichiers à modifier.

Tracked in: senara-solutions/mika#2826 — pour les deux lignes restantes du parapluie, qui ne
sont pas du travail de ce vol.

## Ordre de commit

1. `_classify_revise_halt` + sa doctrine dans `dispatch-lib.sh` (R2) — les deux contraintes
   `unset` et sous-shell, nommées sur place.
2. `_launch_revise_pilot` : lecture de la sortie avant suppression, les trois arms, la
   publication sur les cinq `return 1`, la ligne de R7 sur la branche de succès (R1, R3, R4,
   R5, R7).
3. Le site `:10287` reporte le diagnostic (R6).
4. V1–V15 dans `test-dispatch-lib.sh`, sonde comprise (R8, R9).

Plancher mergeable : les quatre pas. Les pas 1–3 sans le pas 4 livreraient une honnêteté que
rien ne garde — ce que ce parapluie reproche précisément aux six suites non exécutées de sa
première ligne.

## Risques

- **La sonde ne reproduit pas la capture réelle de stdout.** Borné : la redirection
  `>"$revise_stdout"` est posée par `:9260` sur la *commande*, donc elle s'applique au stub
  comme au vrai `_run_pilot_sandboxed`. C'est déjà la mécanique sur laquelle T2306 et S2636
  reposent, et les deux sont vertes en CI depuis leur merge.
- **Le mémo hérité fait un faux vert sur V1.** C'est exactement l'inverse qui est mesuré : V6
  pré-pose `_HALT_SUBTYPE_RESOLVED=error_max_turns` et exige que le diagnostic dise
  `idle_timeout`. Sans le `unset`, V6 rougit — et V13 vérifie qu'elle rougit bien.
- **Un sous-type absent du `case` de `_halt_family`.** Le bras `*)` existe et se nomme
  (`:6143`) : le diagnostic dira `unknown|investigate`, ce qui reste strictement meilleur que
  « plan unchanged ». Aucune table n'est recopiée, donc rien à tenir à jour ici.
- **Le diagnostic rallonge le motif du terminal.** Accepté, et c'est l'intention. `:10381`
  porte déjà un motif d'une longueur comparable, sur une seule ligne, et V7 garde la propriété
  qui compte pour le `sed`.
- **`$revise_stderr` vide quand le pilote n'a rien écrit.** Le repli de `:6252-6256` est
  explicitement fail-open (« stderr only enriches ») : le diagnostic dégrade vers « cause non
  enregistrée » et ne perd jamais la distinction `terminated` / conclu, qui vient du `status`.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — `_classify_revise_halt` + doctrine | 62 |
| `skills/bundled/_shared/dispatch-lib.sh` — `_launch_revise_pilot` (capture, trois arms, publication, R7) | 48 |
| `skills/bundled/_shared/dispatch-lib.sh` — site appelant `:10287` | 8 |
| tests (`skills/bundled/_shared/test-dispatch-lib.sh` — sonde + V1–V15 + deux copies régressées) | 145 |

Total estimé : 263 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

263 < 279, et 263 < 1000 : un seul vol. **Pourquoi il tient** — aucun fichier n'est créé (donc
aucun câblage `Makefile`/`ci.yml`/`suite-wiring.tsv` à produire ni à mesurer, ce qui est le
poste qui avait coûté 7 lignes et deux allers-retours au vol 2) ; le correctif tient dans une
fonction neuve et trois arms d'une fonction existante, sur un site d'appel unique déjà localisé
à la ligne ; la sonde est une transposition d'un harnais qui existe deux fois dans le même
fichier ; et aucun code Rust n'est touché, donc pas de cycle `cargo build`.
