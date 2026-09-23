# mika#2051 — La moitié code est livrée ; la surface qui devait la rendre lisible publie un instrument mort

**Ticket :** mika issue#2051
**Type :** fix (observabilité opérateur + corrélation ; pas de cause inventée)
**Date :** 2026-09-21 (v4 : 2026-09-22)

---

## Problème

### Ce que le ticket demande

Deux proxies d'egress (pids `2383949`, `2387253`) sont morts entre `exec` et
`bind()` le 2026-08-29, sans écrire une ligne, alors que `dispatch-lib` capture
leur stderr. Le ticket isole la question que mika#2041 a refusé de combler par
une hypothèse — **qui les a tués** — et énonce sa propre condition de
résolution :

> « la prochaine occurrence laissera une trace exploitable. C'est la condition
> qui manquait pour enquêter au lieu de subir. **Ce ticket est là pour recevoir
> cette trace.** »

Une trace qui arrive et que personne ne sait lire est la même chose qu'une trace
absente, avec une ligne de journal en plus. **Le livrable de ce ticket est donc
la lisibilité de la trace, pas sa production** — celle-ci est déjà livrée.

### Rectification 1 — la moitié code est déjà livrée et mergée

`ed8d0e2b` / PR #2086, *« fix(egress): name a pre-bind proxy death instead of
dying silent (mika#2051) »*, 2026-08-30. **Re-vérifié ancêtre de `HEAD` au
2026-09-22**, titre exact. Ce qu'elle a posé, et qui tourne depuis trois
semaines :

- `pilot_egress_startup.begin pid=<pid> binding <path>` — première ligne émise,
  **avant** `bind()`, horodatée via `_log` (`scripts/mika-pilot-egress-proxy:1284`) ;
- des handlers SIGTERM/SIGINT armés **avant** `bind()` (`:1269`), qui nomment un
  signal pré-bind et sortent en `3` au lieu de mourir muets ;
- un seam de test (`:1292`) et deux tests de régression (`:1594`, `:1611`).

Le message de commit nomme lui-même ce qui reste ouvert, et il faut le citer
plutôt que le paraphraser :

> « Who SENT the signal (guardrail / timeout / process-group teardown / OOM)
> **is operational and needs host-side evidence not in this repo**; this change
> makes the next occurrence say which of those it was. »

**Le risque principal de ce ticket reste qu'on réimplémente #2086, ou qu'on
invente la cause que le ticket a explicitement refusé d'inventer.** Ce plan
refuse les deux (D1, D5).

### Rectification 2 — mika#2049 a inversé la posture, et v3 de ce plan est périmé

**C'est le livrable principal de cette révision.** Le 2026-09-20, `b86062f8` /
PR #2439 — *« posture fail-closed du relais d'egress »*, décision de Vincent
après bearing de Prime — a changé le comportement que les versions v1–v3 de ce
plan prenaient pour acquis. Trois conséquences, chacune mesurée :

**(1) Le repli fs-only n'existe plus.** `_ensure_pilot_egress_proxy`
(`dispatch-lib.sh:537`) rapporte désormais une cause dans `_PILOT_EGRESS_ABORT`
et `_run_pilot_sandboxed` **refuse le dispatch** (`:1159`, `CONTAINMENT REFUSAL`
exit 78 à `:3118`). Le commentaire de posture (`:517-536`) l'écrit :
*« Egress unavailable ⇒ the dispatch is refused; the pilot never leaves without
its network cut. »* Il n'existe **aucune** variable d'environnement qui lève le
refus.

**→ D6 de v3 est réfuté par événement.** v3 écrivait que « transformer un repli
documenté en refus de dispatch est une décision de politique de containment,
d'un tout autre rayon d'explosion, qui n'est pas dans ce ticket ». Le cadrage
était juste : la décision a été prise, dans son propre ticket, par l'opérateur.
Ce plan en hérite ; il ne la rouvre pas.

**(2) La sous-chaîne `falling back to fs-only` a disparu du code émetteur.**
Chaque cause porte désormais son propre jeton stable :
`pilot_egress_guard.binary_missing` (`:554`), `pilot_egress_guard.unreachable`
(`:594`), `pilot_egress_guard.recovered` (`:686`). Mesure au 2026-09-22 :
`falling back to fs-only` ne subsiste que dans **deux commentaires** qui
expliquent son retrait (`:545`, `:593`), dans `CLAUDE.md` et dans des plans.

**→ U2, V7 et AC3 de v3 sont inatteignables tels qu'écrits.** Ils exigeaient de
préserver une sous-chaîne qui n'existe plus. L'invariant est réécrit en U3.

**(3) `CLAUDE.md` § Signal S n'a pas été mis à jour, et publie un instrument
mort.** Mesure : `grep -c "mika#2049" CLAUDE.md` → **0**. La section décrit le
monde d'avant, à trois endroits :

| ligne | ce qui est publié | état réel |
|---|---|---|
| `:269` | le grep principal, `^dispatch-lib: .*falling back to fs-only` | **ne peut plus rendre une seule ligne** |
| `:269` | « degraded-but-functional — nothing breaks, **no dispatch fails**, no PR goes missing » | l'inverse : le dispatch est refusé, escaladé sur Telegram |
| `:273` | « **two** paths into fs-only », dont `mika-pilot-egress-proxy not found at … (falling back to fs-only)` | les deux chemins refusent, et cette ligne porte désormais `pilot_egress_guard.binary_missing … — refusing the dispatch` |

**Le régime publié de ce grep est « Steady state: zero occurrences ».** Donc un
instrument mort s'y lit **exactement** comme une flotte saine : classe mika#2205
(« un scan silencieusement inactif se lit exactement comme un scan qui n'a rien
trouvé à faire »), sur la section qui porte elle-même la phrase *« a signal
nobody looks for is not an improvement on a silence »*. C'est la **troisième**
correction du Signal S, après les deux de mika#2050.

**Pourquoi rien n'a rougi** : le lint de jetons canoniques (mika#2201,
`scripts/canonical-tokens.tsv`) ne déclare **aucun** jeton egress — mesuré, une
seule occurrence du mot dans le fichier et c'est un commentaire. Les prédicats
opérateur de `CLAUDE.md` sont hors de sa population. Constat qui nomme le suivi,
sans le livrer ici (§ Suivi).

### Rectification 3 — une surface opérateur dédiée existe désormais

mika#2049 a livré `docs/operator/pilot-egress-relay.md`, un runbook à jour :
tokens, symptômes (Telegram / `RESULT` / journaux / SQL), diagnostic en trois
questions, gestes, réglage, haltes. **Il change le choix de surface de v3**
(D2) : la table de signatures doit aller là, pas dans `CLAUDE.md`.

Et il rend **inutile l'U3 de v3** — l'entrée `docs/solutions/` que v3 voulait
créer pour porter l'état de la question et la procédure de mesure. Le runbook
*est* cet artefact, mieux placé. Un second lieu serait un second lieu à
maintenir, ce que v3 refusait déjà pour lui-même. **Le périmètre se resserre
d'une unité alors même qu'il en gagne une.**

### Ce qui reste réellement ouvert

**(Q1) La trace est-elle arrivée ?** Mesure, pas code. **Structurellement
indisponible depuis une session pilote** : le bac à sable bwrap ne monte pas le
journal hôte (classe mika#2165), donc `/var/log/mika/` n'est pas lisible d'ici.
**L'absence de ces fichiers dans un worktree de dispatch ne prouve rien sur
l'hôte** et ne doit jamais être lue comme « aucune récurrence ». Geste
opérateur, prescrit comme tel (U2).

**(Q2) L'instrument livré est-il lisible par quelqu'un qui ne l'a pas écrit ?
Non — et c'est le défaut résiduel, déplacé mais intact.** `pilot_egress_startup`
apparaît dans **zéro** fichier markdown du dépôt hors ce plan (mesuré au
2026-09-22) : ni dans `CLAUDE.md`, ni dans le runbook. Or le runbook §3 Q3
envoie l'opérateur dans le journal du proxy avec un `tail -50` et **s'arrête
là** — sans dire quoi y chercher. Sa Halte 1 désigne ce même fichier comme
*« l'instrument sur lequel mika#2041 puis **mika#2051** se sont appuyés »* : le
runbook nomme ce ticket et ne porte pas sa table. Toute la valeur diagnostique
de #2086 vit dans un commentaire Python (`:1264-1265`).

**(Q3) Peut-on joindre « ce dispatch a été refusé » à « ce proxy est mort » sans
corréler à la main ?** Non, et l'asymétrie est à l'envers de ce qu'il faudrait :

```
594:  echo "dispatch-lib: pilot_egress_guard.unreachable … within 3s — refusing the dispatch (mika#2049)" >&2
597:  echo "dispatch-lib: pilot-egress-proxy launched (pid $!, log $log_file)" >&2
```

**Le chemin de succès porte le pid ; le chemin d'échec — le seul que ce ticket
existe pour diagnostiquer — n'en porte aucun.** Le journal du proxy est
cumulatif, donc la jointure se fait par horodatage entre deux fichiers : la
friction même qui a rendu le diagnostic du 2026-08-29 coûteux. Le proxy porte
son pid dès sa première ligne : **la clé existe des deux côtés, elle n'est pas
imprimée du côté échec.** Et elle vaut plus qu'en v3, puisque le refus est
désormais escaladé : l'opérateur arrive au journal avec un épisode précis à
attribuer.

**(Q4) Qui a tué les deux proxies ?** Hors dépôt, non affirmé (D5).

### Ce qui a été vérifié et n'est PAS un défaut

Écrit pour que la prochaine lecture ne le re-suspecte pas.

- **`$!` à la ligne 597 est fiable.** Entre le `nohup … &` (`:573`) et sa
  lecture, seuls `_pilot_egress_sock_connectable` (`:500`, python3 en
  **avant-plan**) et `sleep` s'exécutent : aucun arrière-plan ne s'interpose.
  U3 le capture tout de même immédiatement, parce qu'il faut la valeur **avant**
  la ligne 594 — la robustesse est un effet de bord, pas le motif.
- **`.begin` est bien la première ligne**, et c'est **déjà verrouillé** par
  `test_startup_emits_a_begin_breadcrumb_before_bind` (`:1594`), qui assert sur
  `proc.stderr.readline()`. La branche « aucune ligne `.begin` » de la table
  repose donc sur un invariant testé, pas sur une convention. **Aucun test à
  ajouter de ce côté.**
- **Les hypothèses que le ticket réfute restent réfutées** : le déliement de
  socket éventée (`:1299`) tourne bien, `stat` est importé (`:66`).
- **Le harnais de test a suivi mika#2049 et reste utilisable.**
  `_egress_guard_probe` (`test-dispatch-lib.sh:4716`) a été re-tokenisé
  (`binary-missing` / `unreachable` / `launched-ok` / `none`) et ses arms sont
  désormais disjoints par construction plutôt que par ordre. Il exerce toujours
  le vrai `_ensure_pilot_egress_proxy` contre un faux proxy qui meurt avant de
  binder — **aucun second harnais à écrire.**
- **`make verify-egress-no-log` est un faux voisin, et il faut le dire avant
  qu'il ne fasse hésiter.** La cible (`Makefile:200`) scanne
  `crates/mika-gateway/src/egress_search/` — le substrat de **recherche** egress
  — et non le proxy du pilote. Le runbook ouvre d'ailleurs sur cet
  avertissement. **Conséquence pratique : la discipline no-log ne s'applique pas
  à U3**, dont tout le livrable est d'ajouter de l'information à une ligne de
  journal. Un implémenteur qui confond les deux s'auto-censurerait sur le seul
  livrable de code du plan.

---

## Requirements

- **R1** — `CLAUDE.md` § Signal S cesse de publier un prédicat qui ne peut plus
  mordre et une prose qui décrit le comportement inverse de celui en vigueur, et
  renvoie à la surface canonique.
- **R2** — Le runbook porte la table de signatures à quatre branches, de sorte
  qu'un opérateur envoyé dans le journal du proxy sache quoi y chercher et ce que
  chaque forme veut dire.
- **R3** — La ligne d'échec du lanceur nomme le pid du proxy qu'elle vient de
  lancer, pour que la jointure avec `pilot_egress_startup.begin pid=…` soit
  exacte plutôt que temporelle.
- **R4** — La mesure Q1 est prescrite comme geste **hôte**, avec la raison pour
  laquelle elle n'est pas prenable depuis un pilote.
- **R5** — Aucune cause n'est affirmée, aucune n'est écartée sans mesure. Le plan
  ne referme pas Q4 et dit pourquoi.
- **R6** — Aucune régression du comportement d'exécution : ni le proxy, ni la
  posture fail-closed, ni les codes de retour, ni les jetons ne changent.

---

## Décisions

- **D1 — Ne rien réimplémenter de #2086, et le dire en tête de plan.** La
  tentation structurelle est de relire le corps du ticket (« deux proxies sont
  morts, rien dans le log ») et de livrer l'instrumentation… qui est là depuis
  trois semaines. Le corps a été écrit **avant** le correctif et n'a pas été
  amendé. C'est la classe mika#2340 transposée à un ticket : *établir l'état
  déployé avant de toucher au code.*

- **D2 — Deux surfaces, deux livrables différents, et une seule source de
  vérité.** Le runbook `docs/operator/pilot-egress-relay.md` est la surface
  canonique depuis mika#2049 : il porte les tokens, le diagnostic et les haltes,
  et c'est lui qui envoie l'opérateur dans le journal du proxy. **La table de
  signatures y va.** `CLAUDE.md` § Signal S n'est pas le lieu de la dupliquer —
  mais il ne peut pas rester tel quel : il publie un grep mort sous un régime
  « zéro attendu ». Il est donc **corrigé, pas enrichi**, et renvoie au runbook.
  Écrire la table aux deux endroits créerait deux vérités dont l'une dériverait,
  ce qui est précisément la panne que cette révision documente.

- **D3 — La table est à quatre branches, pas trois.** Le commentaire du code en
  nomme trois (`:1264-1265`). La quatrième — **aucune ligne `.begin`** — est la
  plus importante pour un opérateur, parce qu'elle dit « le processus est mort
  avant que Python ne tourne » (échec d'`exec`, interpréteur, dépendance) et
  qu'elle est la seule qui se lit par une **absence**. La laisser implicite,
  c'est laisser l'opérateur conclure « le journal ne dit rien » là où le journal
  dit quelque chose de précis.

- **D4 — Le pid sur la ligne d'échec, et pas un identifiant de corrélation
  nouveau.** Le réflexe serait d'inventer un `egress_correlation_id` posé des
  deux côtés. Refusé : la clé existe déjà et elle est imprimée du côté proxy
  depuis #2086. Un identifiant nouveau serait un second vocabulaire pour joindre
  ce que le pid joint déjà, à propager à travers `nohup`. Une variable locale
  capturée juste après le lancement suffit.

- **D5 — Ne pas répondre à « qui a envoyé le signal ».** Le ticket refuse
  explicitement d'inventer, le commit de #2086 le refuse aussi, et la mesure est
  hors dépôt. Ce plan hérite du refus. Il livre de quoi **attribuer** à la
  prochaine occurrence, pas une cause.

- **D6 — La posture fail-closed est héritée, pas rouverte.** v3 déclarait le
  sujet hors périmètre ; mika#2049 l'a tranché dans son propre ticket, sur
  décision opérateur, avec son coût écrit (*« une boucle arrêtée est réversible
  et visible ; un egress ouvert sur du code auto-écrit ne l'est pas »*). Ce plan
  n'ajoute aucune garde, ne touche aucun code de retour et n'introduit aucune
  échappatoire — l'option en avait été écartée par écrit.

- **D7 — Aucun `docs/solutions/` nouveau.** v3 en prévoyait un pour porter
  l'état de la question et la procédure de mesure. Le runbook mika#2049 le fait
  déjà et mieux ; ce plan l'y complète au lieu d'ouvrir un second lieu.

- **D8 — Ne pas livrer la garde anti-divergence ici.** Ce qui a permis à la
  divergence du Signal S de passer est l'absence des prédicats opérateur dans
  `scripts/canonical-tokens.tsv`. Étendre ce lint serait un détecteur avec une
  **population existante** (d'autres prédicats publiés peuvent être morts), donc
  une Fire-Disposition à part entière et un rayon d'explosion propre. Ce plan
  corrige la divergence mesurée et **nomme** le suivi ; il ne livre pas une
  garde dont il n'a pas mesuré la population.

---

## Scope Boundaries

**Dans le périmètre :**
- `CLAUDE.md` § Signal S — correction du prédicat mort et de la prose inversée.
- `docs/operator/pilot-egress-relay.md` §3 Q3 — table de signatures, clé de
  jointure, note « geste hôte ».
- `skills/bundled/_shared/dispatch-lib.sh` — pid sur la ligne d'échec.
- `skills/bundled/_shared/test-dispatch-lib.sh` — assertion sur ce pid.

**Hors périmètre, délibérément :**
- **`scripts/mika-pilot-egress-proxy`** — la moitié code est livrée et testée.
  Toute modification y serait une réimplémentation de #2086 (D1).
- **La cause du signal** (guardrail, timeout, teardown de groupe de processus,
  OOM) — D5.
- **La posture fail-closed, ses jetons, ses codes de retour, son escalade** —
  D6. Ce plan ajoute un champ à une ligne, il ne change aucune décision.
- **L'extension du lint mika#2201 aux prédicats opérateur** — D8.
- **Le relais wedgé non détecté**, `bwrap` absent du `PATH`, `MIKA_PILOT_SANDBOX=0`
  — trois trous réels déjà nommés au §8 du runbook, chacun avec son préalable.
- **Le sink du Signal M** (`pilot_push_guard`, qui n'atterrit dans aucun
  fichier) — défaut réel, voisin, déjà nommé dans `CLAUDE.md` comme suivi.

---

## Implementation Units

### U1 — Le Signal S cesse de publier un instrument mort (R1)

`CLAUDE.md`, § *Signal S* (`:269-276`). Trois corrections, aucune addition de
table :

1. **`:269` — le prédicat.** `^dispatch-lib: .*falling back to fs-only` ne mord
   plus sur rien. Le remplacer par le prédicat qui couvre la population
   d'aujourd'hui, ancré comme l'exige `:271` :
   `^dispatch-lib: pilot_egress_guard\.` — qui capte les trois jetons
   (`binary_missing`, `unreachable`, `recovered`). Le discriminant par cause
   reste le jeton, comme `:273` l'enseigne déjà.
2. **`:269` — la prose.** « degraded-but-functional — nothing breaks, no
   dispatch fails, no PR goes missing » décrit le régime que mika#2049 a
   supprimé. Le régime actuel est l'inverse et il est **bruyant** : dispatch
   refusé, `CONTAINMENT REFUSAL` exit 78, escalade Telegram. La phrase qui la
   remplace doit dire que **le silence n'est plus la forme de cette panne**, ce
   qui change la conduite de l'opérateur.
3. **`:273` — « two paths into fs-only ».** Il n'y a plus de chemin vers
   fs-only. Les deux causes subsistent et restent distinctes (le runbook §3 Q1
   le dit), mais elles refusent toutes deux. La leçon de mika#2050 —
   *« zéro occurrence de `pilot_egress_guard.unreachable` ne prouve pas que la
   coupure réseau est active »* — **reste vraie et doit être conservée** : c'est
   le `binary_missing` qui la porte désormais.

Ce qui **ne bouge pas** : l'ancre (`:271`), le contrôle positif (`:272`, dont la
ligne `pilot-egress-proxy launched` est inchangée à `dispatch-lib.sh:597`), les
deux haltes, la limite du chemin revise. La **Remedy** (`:274`) gagne un renvoi
au runbook comme surface canonique — c'est là que la table vit (D2).

### U2 — Le runbook dit quoi chercher dans le journal du proxy (R2, R4)

`docs/operator/pilot-egress-relay.md`, §3 Q3 — qui aujourd'hui prescrit
`tail -50` sur les deux chemins et s'arrête. Les deux chemins restent (leur
raison d'être — *« ne chercher que le premier est la façon dont on conclut "pas
de journal, donc rien n'a tourné" »* — est intacte). Ajouter la lecture :

```bash
grep -E 'pilot_egress_startup|host-unix listening on' \
  "${MIKA_PILOT_EGRESS_LOG_DIR:-/var/log/mika}/pilot-egress-proxy.log"
```

| Ce qu'on lit pour un lancement | Lecture |
|---|---|
| `.begin` **puis** `host-unix listening on` | sain — ce proxy a bindé |
| `.begin`, **pas** de `.signalled`, **pas** de `listening` | mort dans la fenêtre pré-bind par un signal **non rattrapable** — SIGKILL ou OOM-kill |
| `.begin` **puis** `.signalled <SIG>` (sortie `3`) | un SIGTERM/SIGINT a atterri pendant le démarrage — le signal est nommé |
| **aucune** ligne `.begin` pour ce lancement | mort **avant** que Python ne tourne — échec d'`exec`, interpréteur, dépendance manquante |

Trois phrases à écrire avec le tableau, parce que ce sont elles qui empêchent la
mauvaise conclusion :

- l'absence de `.begin` **pour un lancement donné** est une information, pas un
  silence — à ne pas confondre avec un journal **absent**, qui est le cas
  « binaire jamais déployé » que le §3 Q1 traite déjà ;
- `.begin` est la **première** ligne du processus par contrat testé, donc la
  quatrième branche est lisible ; si un jour elle cesse de l'être, c'est
  `test_startup_emits_a_begin_breadcrumb_before_bind` qui rougit ;
- **la jointure** : le pid de la ligne `pilot_egress_guard.unreachable` du
  `.stderr` du dispatch se retrouve tel quel dans le `pid=` du `.begin` du
  journal du proxy (rendu exact par U3).

Et la note Q1, qui appartient au même §3 : **cette lecture est un geste hôte.**
Un pilote dispatché ne voit pas `/var/log/mika/` (bwrap ne le monte pas, classe
mika#2165) ; l'absence du fichier depuis une session de dispatch n'est **pas**
un résultat et ne doit jamais être lue comme « aucune récurrence ».

### U3 — La ligne d'échec nomme le pid qu'elle a lancé (R3)

`skills/bundled/_shared/dispatch-lib.sh`, `_ensure_pilot_egress_proxy` :

- capturer `local proxy_pid=$!` **immédiatement** après le `nohup … &` /
  `disown` (`:573-575`) ;
- `:594` (`pilot_egress_guard.unreachable`) : ajouter `(pid <proxy_pid>)` ;
- `:597` (succès) : lire `$proxy_pid` au lieu de `$!` — même valeur aujourd'hui
  (vérifié : aucun arrière-plan ne s'interpose), mais la lecture ne dépend plus
  d'un invariant à distance.

**L'invariant à tenir, et il a changé avec mika#2049.** Les prédicats publiés
qui mordent sur cette famille de lignes sont désormais ceux du runbook §2
(`:77-80`) et du Signal S corrigé :

| prédicat publié | ce qu'il couvre |
|---|---|
| `^dispatch-lib: pilot_egress_guard.unreachable` | le relais ne bind pas |
| `^dispatch-lib: pilot_egress_guard.binary_missing` | le binaire n'est pas déployé |
| `^dispatch-lib: pilot-egress-proxy launched` | le contrôle positif |

L'invariant est donc : **l'ancre `^dispatch-lib: ` reste en tête et le jeton
reste immédiatement après, contigu.** Il n'y a plus de sous-chaîne de fin de
ligne à préserver — la contrainte que v3 portait sur `falling back to fs-only`
est éteinte avec elle. Tout point d'insertion après le jeton est conforme.

Forme retenue — le pid **avant** le tiret cadratin, comme un choix motivé et non
comme la seule option conforme :

```
dispatch-lib: pilot_egress_guard.unreachable pilot-egress-proxy failed to bind <sock> within 3s (pid <proxy_pid>) — refusing the dispatch (mika#2049)
```

La raison est de lecture : le tiret sépare le **constat** (ce proxy-là,
identifié, n'a pas bindé) de sa **conséquence** (le dispatch est refusé). Le pid
qualifie le constat ; le poser après l'attacherait à la conséquence, qui ne lui
appartient pas. V7 vérifie l'invariant, jamais la position.

**Et la ligne 554 ne bouge pas.** La voie « binaire absent » n'a lancé aucun
proxy, donc elle n'a pas de pid à nommer. Lui en inventer un serait une
affirmation fausse.

**Le harnais de test existe et jette la ligne — c'est la contrainte qui décide
la forme de V3.** `_egress_guard_probe` (`test-dispatch-lib.sh:4716`) exerce
déjà le vrai `_ensure_pilot_egress_proxy` contre un faux proxy qui meurt avant
de binder — la population exacte dont V3 a besoin, donc **aucun second harnais à
écrire**. Mais il **classe** la sortie en un jeton (`:4797-4802`) puis jette le
texte : il ne rend que `rc=… launched=… msg=…` (`:4804`). Et **cinq assertions
comparent cette chaîne par égalité stricte** (`:4809`, `:4815`, `:4820`,
`:4828`, `:4834`), donc même un ajout en fin de chaîne les fait toutes rougir.
Deux voies, et le plan retient la seconde :

- étendre `_egress_guard_probe` et mettre à jour les cinq assertions — cinq
  lignes touchées pour un besoin qui n'en concerne qu'une ;
- **ajouter une fonction sœur** qui rend la ligne brute (`_egress_guard_line`,
  même fabrication de faux proxy, retour non classé). Aucune assertion existante
  n'est touchée, et la séparation dit ce qu'elle fait : l'une teste la
  **décision**, l'autre le **texte**.

Deux invariants du harnais à ne pas casser en le doublant : la redirection de
`MIKA_PILOT_EGRESS_LOG_DIR` vers un temporaire — sans elle la sortie du faux
proxy atterrit dans le journal opérationnel, celui-là même que ce ticket existe
pour rendre lisible, et l'assertion `:4839-4840` le vérifie ; et le fait que les
arms du `case` sont désormais **disjoints par jeton** (mika#2049) et non plus
ordonnés défensivement — U3 ne touche que la ligne 594, donc ce classement reste
exact.

---

## Verification Contract

- **V1** — `grep -n "pilot_egress_startup" docs/operator/pilot-egress-relay.md`
  retourne au moins une ligne. **Le prédicat porte sur le runbook, jamais sur
  `*.md` en général** : ce plan contient lui-même le jeton une dizaine de fois,
  donc un `grep -rn … --include="*.md" .` serait satisfait par le plan seul et
  n'attesterait rien de la surface opérateur. Contrôle négatif : avant ce
  travail, le runbook en porte **zéro** (mesuré 2026-09-22).
- **V2** — `grep -c "falling back to fs-only" CLAUDE.md` retourne **0**.
  Contrôle négatif : avant ce travail, **2** (`:269`, `:273`).
- **V3** — `make test-dispatch-lib` passe, assertion U3 comprise.
- **V4** — Nouvelle assertion dans `test-dispatch-lib.sh`, portée par une
  fonction sœur de `_egress_guard_probe` qui rend la ligne brute : sur un chemin
  de socket qui ne bindera jamais, la ligne `pilot_egress_guard.unreachable`
  contient un pid numérique **et** garde son ancre `^dispatch-lib: ` avec son
  jeton contigu.
- **V5 — contrôle négatif de V4** : sans le changement U3, l'assertion rougit.
  Sans lui, V4 ne distingue pas « le pid est imprimé » de « l'assertion est
  triviale ».
- **V6** — Les cinq assertions existantes de `_egress_guard_probe` (`:4809`,
  `:4815`, `:4820`, `:4828`, `:4834`) passent **inchangées**, et l'assertion
  `:4839-4840` (« aucune sortie de faux proxy dans le journal opérationnel »)
  aussi. C'est le contrôle que la fonction sœur n'a pas été obtenue en cassant
  le harnais qu'elle double.
- **V7** — Les prédicats publiés mordent sur la sortie produite en V4 :
  `^dispatch-lib: pilot_egress_guard.unreachable` et, sur la voie binaire
  absent, `^dispatch-lib: pilot_egress_guard.binary_missing`. Le prédicat porte
  sur l'invariant — ancre en tête, jeton contigu — **jamais sur la position du
  pid** : une assertion sur la position figerait une forme de rédaction là où ce
  qui compte est ce sur quoi les greps mordent.
- **V8** — `scripts/test-pilot-egress-proxy-status.py` passe **inchangé** et
  `git diff --stat` ne liste pas `scripts/mika-pilot-egress-proxy` (contrôle de
  D1 / hors-périmètre).
- **V9** — Aucun jeton, aucun code de retour, aucune décision de refus ne change :
  `git diff` sur `dispatch-lib.sh` ne touche ni `:554`, ni `:686`, ni `:1159`,
  ni les constantes `_PILOT_EGRESS_MOTIF_*` (`:313-314`).

---

## Definition of Done

- Le § Signal S ne publie plus de prédicat mort ni de prose inversée, et renvoie
  au runbook (V1, V2).
- Le runbook porte la table de signatures, la clé de jointure et la note « geste
  hôte » ; le grep V1 n'est plus vide.
- La ligne d'échec du lanceur porte le pid ; V4 et son contrôle négatif V5
  passent.
- Le proxy n'est pas modifié (V8), la posture fail-closed n'est pas touchée (V9).
- Le corps du ticket est amendé ou commenté par l'opérateur pour dire que la
  moitié code est livrée par #2086 et que la posture a changé avec mika#2049 —
  **geste opérateur, hors de ce plan** (le pilote de grooming n'écrit pas sur le
  ticket).

---

## Acceptance criteria

Dérivées du plan : le corps du ticket ne porte pas de section
`## Acceptance criteria`.

- **AC1** — `grep -n "pilot_egress_startup" docs/operator/pilot-egress-relay.md`
  retourne au moins une ligne. (Avant : zéro. Le prédicat est ancré sur le
  runbook et non sur `*.md` — voir V1 pour pourquoi une version élargie
  s'auto-satisferait.)
- **AC2** — Le §3 Q3 du runbook nomme les quatre branches de la table, y compris
  la branche « aucune ligne `.begin` », nomme le pid comme clé de jointure entre
  le `.stderr` du dispatch et le journal du proxy, et dit que la lecture est un
  geste hôte.
- **AC3** — `grep -c "falling back to fs-only" CLAUDE.md` retourne `0`, et le §
  Signal S ne décrit plus le régime « no dispatch fails » ni « two paths into
  fs-only ». La leçon mika#2050 (« zéro `unreachable` ne prouve pas que la
  coupure est active ») est **conservée**, portée par `binary_missing`.
- **AC4** — Sur un échec de bind, la ligne `pilot_egress_guard.unreachable`
  émise par `_ensure_pilot_egress_proxy` contient un pid numérique, tout en
  conservant l'ancre `^dispatch-lib: ` et son jeton contigu.
- **AC5** — `make test-dispatch-lib` passe ; l'assertion AC4 rougit si le
  changement est retiré ; les six assertions préexistantes du harnais egress
  passent inchangées.
- **AC6** — `scripts/mika-pilot-egress-proxy` est **inchangé** par ce travail, et
  `scripts/test-pilot-egress-proxy-status.py` passe sans modification.
- **AC7** — Aucun livrable n'affirme une cause de la mort des deux proxies du
  2026-08-29, et aucun ne change la posture fail-closed, ses jetons, ses codes de
  retour ou son escalade.

---

## Fire-Disposition

**Option (a) — aucune exception nécessaire, et c'est vérifié plutôt
qu'affirmé.**

Ce plan livre **un** détecteur : l'assertion V4/AC4 dans
`skills/bundled/_shared/test-dispatch-lib.sh`, dont le chemin de succès est
« la ligne d'échec porte un pid ».

Il ne peut pas firer sur des données existantes : il n'inspecte aucun corpus,
aucun historique, aucun fichier du dépôt. Il exécute `_ensure_pilot_egress_proxy`
contre un chemin de socket fabriqué qui ne bindera jamais, et lit la ligne
produite dans la foulée. Sa population est donc **créée par le test lui-même**, à
chaque exécution — il n'y a pas de violation préexistante possible, donc **aucune
exception à allowlister**. L'allowlist est vide et doit le rester ; si ce
détecteur rougit un jour, c'est que la ligne a cessé de porter le pid, et la
résolution est de le rendre, jamais de l'exempter.

U1 et U2 sont documentaires (une section de `CLAUDE.md`, un § du runbook) :
aucune n'a pour fonction primaire de signaler une violation, aucune ne peut
firer. La garde qui **aurait** une population existante — l'extension du lint
mika#2201 aux prédicats opérateur — est explicitement hors périmètre (D8) et
renvoyée au suivi, précisément parce qu'elle appellerait sa propre disposition.

**Le contrôle négatif V5 est ce qui rend cette disposition honnête** : sans lui,
une assertion triviale passerait pour un détecteur armé — exactement la panne que
mika#2272 a nommée (« zéro était l'absence de mesure, pas la présence de
prudence »).

---

## Suivi (hors périmètre, nommé)

- **La cause de la mort des deux proxies du 2026-08-29** reste **ouverte** et ne
  se ferme pas en dépôt (D5). Elle se ferme sur une récurrence, attribuée par la
  table d'U2. **Halte explicite : si aucune récurrence n'est mesurée, la
  conclusion est « instrumenté, sans récurrence » — pas « cause identifiée ».**
  Le ticket peut être fermé sur cette base, ce qui est un résultat et non un
  abandon.
- **Étendre `scripts/canonical-tokens.tsv` aux prédicats opérateur publiés**
  (Signal S, Signal Q, runbooks). C'est l'absence de cette déclaration qui a
  laissé mika#2049 retirer une chaîne grepée par `CLAUDE.md` sans rien faire
  rougir, et c'est la **troisième** divergence du Signal S. **Préalable écrit :
  mesurer la population** — combien de prédicats publiés dans `CLAUDE.md` ne
  mordent plus aujourd'hui ? Livrer la garde avant cette mesure, c'est ignorer sa
  propre Fire-Disposition.
- **Halte de mesure** — si la mesure hôte montre des `.begin` **sans**
  `listening` alors qu'aucun `pilot_egress_guard.unreachable` n'apparaît côté
  dispatch, ne pas élargir la table : les deux instruments se contredisent et
  c'est **le sink** qu'il faut établir d'abord (`PILOT_LOG_DIR` /
  `MIKA_PILOT_LOG_DIR`, halte 1 déjà écrite au Signal S).
- **Halte de lecture** — `grep` vide dans le journal du proxy **et** journal
  absent sont deux états différents : le second est le cas « binaire jamais
  déployé » (classe mika#2340, et §3 Q1 du runbook), à établir avant toute
  conclusion sur l'egress.
- **Limite héritée, non refermée ici** : `_launch_revise_pilot` redirige son
  stderr vers un `mktemp` qu'il supprime, donc le contrôle Signal S ne couvre pas
  la voie revise. Déjà écrit dans `CLAUDE.md`.
- **Les trois trous du §8 du runbook** (relais wedgé non détecté, `bwrap` absent
  du `PATH`, `MIKA_PILOT_SANDBOX=0`) — chacun avec son préalable, aucun rouvert
  ici.
- **Amender le corps de mika#2051** pour dire que la moitié code est livrée par
  #2086 et que la posture a changé avec mika#2049 : geste opérateur. Sans lui, la
  prochaine lecture du ticket repart sur la trajectoire que D1 refuse — comme
  cette révision a dû le faire pour v3.

---

## Références

- `ed8d0e2b` — PR #2086, *fix(egress): name a pre-bind proxy death instead of
  dying silent (mika#2051)*, 2026-08-30 : la moitié code, déjà livrée.
- `b86062f8` — PR #2439, *posture fail-closed du relais d'egress (mika#2049)*,
  2026-09-20 : ce qui a périmé v3 de ce plan.
- `scripts/mika-pilot-egress-proxy:1264-1265` — la table de signatures, dans un
  commentaire Python ; `:1269` handler précoce, `:1284` `.begin`, `:1292` seam de
  test, `:1299` déliement de socket éventée, `:1325` `host-unix listening on`.
- `scripts/test-pilot-egress-proxy-status.py:1594`, `:1611` — les deux tests de
  régression de #2086.
- `skills/bundled/_shared/dispatch-lib.sh:517-536` — le commentaire de posture
  fail-closed ; `:537` la fonction ; `:554` `binary_missing` ; `:573-575` le
  `nohup`/`disown` ; `:594` `unreachable` (sans pid) ; `:597` `launched`
  (avec pid) ; `:686` `recovered` ; `:1159` le refus ; `:3118` `CONTAINMENT
  REFUSAL`.
- `skills/bundled/_shared/dispatch-lib.sh:500` —
  `_pilot_egress_sock_connectable`, avant-plan : pourquoi `$!` tient encore.
- `skills/bundled/_shared/test-dispatch-lib.sh:4699` `_egress_log_fixture_hits`,
  `:4716` `_egress_guard_probe`, `:4797-4802` le classement par jeton, `:4804`
  le `printf`, `:4809`/`:4815`/`:4820`/`:4828`/`:4834` les cinq assertions à
  égalité stricte, `:4839-4840` la garde de non-pollution du journal.
- `docs/operator/pilot-egress-relay.md` — le runbook mika#2049 : §2 les greps
  publiés (`:73-88`), §3 Q3 le `tail` sans lecture (`:145-154`), §7 Halte 1 qui
  nomme mika#2051, §8 ce qui reste ouvert.
- `CLAUDE.md:269` — le grep mort et la prose inversée ; `:271` l'ancre ; `:272`
  le contrôle positif (intact) ; `:273` « two paths into fs-only » ; `:274` la
  Remedy.
- `scripts/canonical-tokens.tsv` — mika#2201 : zéro jeton egress déclaré,
  pourquoi rien n'a rougi.
- `Makefile:160` `test-dispatch-lib` ; `:200` `verify-egress-no-log` (faux
  voisin, porte sur `crates/mika-gateway/src/egress_search/`).
- mika#2041 — la garde qui rendait cette classe muette ;
  `docs/solutions/best-practices/a-guard-must-observe-not-assert-2026-08-29.md`.
- mika#2050 — les deux corrections de sink sur les Signaux Q et S : précédent de
  forme pour U1, et la leçon `binary_missing` à conserver.
- mika#2165 — le bac à sable ne monte pas le journal : pourquoi Q1 est un geste
  hôte.
- mika#2205 — un instrument silencieusement inerte se lit comme une flotte
  saine : la classe du défaut Q4.

---

## Revision history

- **v1 (2026-09-21)** — Plan initial. Rectification du périmètre : la moitié code
  est livrée par #2086 ; le résiduel en dépôt est la lisibilité opérateur de
  l'instrument et la jointure pid absente du chemin d'échec.
- **v2 (2026-09-21)** — Re-groom sur `3b177df3`. Sept assertions re-confrontées,
  toutes tenues. Trois resserrages de précision, dont une affirmation trop forte
  sur le point d'insertion du pid.
- **v3 (2026-09-21)** — Re-groom sur `4e521a87`. Correction de l'affirmation trop
  forte de v2 ; lecture du harnais de test existant (`_egress_guard_probe` classe
  et jette la ligne, cinq assertions à égalité stricte, d'où la fonction sœur) ;
  `verify-egress-no-log` identifié comme faux voisin.
- **v4 (2026-09-22)** — Re-groom sur `8dd66349`. **Les assertions de v3 ont été
  re-confrontées et l'une des plus portantes est tombée : mika#2049
  (`b86062f8`, PR #2439, 2026-09-20) a inversé la posture d'egress entre-temps.**
  Ce que la re-mesure a établi, et qui change le plan :
  **(a) Le repli fs-only n'existe plus** — le dispatch est refusé (`CONTAINMENT
  REFUSAL`, exit 78, escalade Telegram). **D6 de v3 est réfuté par événement** :
  il déclarait ce changement hors périmètre, l'opérateur l'a tranché ailleurs.
  Le plan en hérite sans le rouvrir.
  **(b) `falling back to fs-only` a disparu du code émetteur** et ne subsiste que
  dans deux commentaires qui expliquent son retrait. **U2, V7 et AC3 de v3
  étaient inatteignables** : ils exigeaient de préserver cette sous-chaîne.
  L'invariant d'U3 est réécrit sur l'ancre et le jeton, les seuls prédicats que
  le runbook publie aujourd'hui.
  **(c) Défaut nouveau et plus grave — `CLAUDE.md` § Signal S publie un
  instrument mort.** `mika#2049` apparaît **zéro** fois dans `CLAUDE.md` : son
  grep principal ne peut plus rendre une ligne, sa prose décrit le régime inverse
  (« no dispatch fails ») et son §273 décrit deux chemins vers un fs-only qui
  n'existe plus — le tout sous un régime publié « zéro attendu », donc **un
  instrument mort s'y lit exactement comme une flotte saine** (classe mika#2205),
  sur la section qui porte elle-même la phrase « a signal nobody looks for is not
  an improvement on a silence ». Troisième divergence du Signal S après les deux
  de mika#2050. Nouvelle unité U1.
  **(d) Pourquoi rien n'a rougi** — `scripts/canonical-tokens.tsv` (mika#2201) ne
  déclare aucun jeton egress. Constat qui **nomme** le suivi (D8) sans livrer une
  garde dont la population n'est pas mesurée.
  **(e) La surface canonique a changé** — `docs/operator/pilot-egress-relay.md`
  (runbook mika#2049) porte tokens, diagnostic et haltes, et sa Halte 1 nomme
  mika#2051 ; mais son §3 Q3 envoie l'opérateur dans le journal du proxy sans
  dire quoi y lire. **D2 est réécrit** : la table va au runbook, `CLAUDE.md` est
  corrigé et renvoie. **D7 nouveau** : l'entrée `docs/solutions/` de v3 est
  **supprimée du périmètre**, le runbook étant déjà cet artefact — le plan perd
  une unité tout en en gagnant une.
  **(f) Numéros de ligne** — tous re-relevés : `dispatch-lib` 531/534 → 594/597,
  `nohup` 512 → 573, harnais 4437 → 4716, tests proxy 1450/1467 → 1594/1611,
  `Makefile` 158/191 → 160/200. Le proxy lui-même (1269/1284/1292/1299/1325) est
  **inchangé**, ce qui confirme D1.
  **Ce qui n'a pas bougé** : D1 (ne pas réimplémenter #2086), D5 (ne pas inventer
  la cause), le refus d'affirmer sans mesure, et la Fire-Disposition (un seul
  détecteur, population créée par le test, allowlist vide).
