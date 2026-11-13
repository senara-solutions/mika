# fix(loop-substrate) — vol 2 : un démon qui porte du code d'egress ne survit plus à son code

- **Ticket :** senara-solutions/mika#2029 (`bug`, `p1-important`, `loop-substrate`, `dispatch:loop`, jalon « Substrat de boucle »)
- **Branche :** `fix/2029/loop-substrate-the-pilot-stall-that` (repartie de `main`, `0/0` contre `origin/main` au moment du groom)
- **Date :** 2026-10-16
- **Vol :** 2 sur 3. **Prédécesseur :** `2026-10-15-002-fix-2029-le-kill-sans-production-se-compte-par-son-nom-plan.md` (vol 1, mergé par #2720) — ce vol reprend son **livrable 3**, qu'il avait explicitement renvoyé (`§ Hors périmètre`, `Tracked in: mika#2029`).
- **Lignée :** mika#1772 → mika#2035 (les AC) → cpp#177 (la cause du stall, côté pilote, **fermée**) → mika#2049 (la posture *fail-closed* du relais) → mika#2716 / mika#2723 / mika#2724 (la sentinelle et son exigence côté addon) → **ce vol** (le cycle de vie des démons qui portent ce code)

---

## Contexte

### Le défaut, en une phrase

Les deux lanceurs de démons hôtes de `dispatch-lib.sh` demandent *« quelqu'un
écoute-t-il ? »* et jamais *« quelle version écoute ? »*. Un démon lancé une
fois sert indéfiniment, y compris après que `make install` a posé un code
différent à côté de lui. Les contrôles livrés par mika#2724 (l'addon exige la
sentinelle) et mika#2723 ne sont donc **pas en vigueur** alors que leur CI est
verte — l'écart le plus coûteux qu'un dépôt puisse porter : un contrôle qui
existe, qui est testé, et qui ne tourne pas.

### Ce que j'ai lu dans le code, aujourd'hui

**M1 — Les deux sondes de vivacité retournent `0` sans poser la question du
code.** `_ensure_pilot_egress_proxy` (`dispatch-lib.sh:1319-1321`) :
`if _pilot_egress_sock_connectable …; then return 0  # already alive`.
`_ensure_pilot_helper` (`:1188-1200`) : même forme, sur un `connect()` TCP vers
`127.0.0.1:8892`. Les deux sont *idempotents par vivacité*, et c'est
exactement ce qui fait survivre un démon à son code.

**M2 — Le démon qui porte les addons est le `mitmdump`, et son code est trois
fichiers.** `:1063-1069` : `$HOME/.local/bin/mitmdump`,
`mika-pilot-anthropic-auth-addon.py`, `mika-pilot-github-auth-addon.py`. Les
deux addons sont chargés dans **le même** processus (`--scripts` répété,
`:1205-1208`). C'est ce processus que mika#2724 a modifié et qui ne l'a pas
appris.

**M3 — `make install` réécrit ces fichiers à chaque déploiement, même sans
changement de contenu.** `Makefile:44-57` : `cp` vers `.tmp`, `chmod`, `mv`
par-dessus la cible, inconditionnellement, pour les trois fichiers d'egress.
Donc **toute** comparaison de mtime produira un verdict « périmé » après
*chaque* deploy, contenu identique ou non. Le vol 1 avait déjà nommé ce piège ;
la décision D3 ci-dessous en fait un choix assumé plutôt qu'un angle mort.

**M4 — Le geste opérateur existant ne couvre pas ce démon.**
`scripts/canary-pilot-containment --restart-relay` (`:291-327`) tue **par
identité de socket** (`fuser -k "$_PILOT_EGRESS_SOCK"`), puis relance le
relais. Il ne connaît ni le port 8892 ni le `mitmdump` qui porte les addons.
C'est le constat du corps du ticket, vérifié ligne à ligne.

**M5 — Le registre de sentinelles ne peut pas servir de signal « un pilote
vole ».** Sa TTL est de **72 h** (`mika-pilot-github-auth-addon.py:166-169`,
« _A registry entry outlives its pilot by at most three days_ ») et rien ne
retire une entrée en fin de session. Une entrée vivante ne dit donc rien de
l'instant. Le signal retenu est ailleurs (D4).

**M6 — Le répertoire de journaux du pilote est déjà un point d'injection
testable.** `_pilot_log_dir` (`:246-248`) vaut
`${PILOT_LOG_DIR:-/var/log/claude-pilot}`. Une sonde qui lit ce répertoire est
redirigeable par une variable que la suite de tests contrôle déjà.

**M7 — Le fixture de test à copier existe.** `_egress_guard_probe`
(`test-dispatch-lib.sh:5732-5830`) construit un faux démon, un vrai écouteur,
redirige le journal opérationnel, source `dispatch-lib.sh` dans un sous-shell
et surcharge les chemins. Six assertions d'une ligne s'appuient dessus. Le
contrôle positif et le contrôle négatif que le ticket exige prennent cette
forme, sans nouvel outillage.

**M8 — Aucune entrée de recensement n'est due.** La famille de jetons voisine,
`pilot_egress_guard.*`, est **absente** de `scripts/canonical-tokens.tsv` : le
recensement mika#2201 porte sur les jetons qu'un lecteur **strict** apparie, pas
sur les lignes de diagnostic que `dispatch-lib.sh` écrit sur sa propre stderr.
La nouvelle famille n'en demande donc pas, et l'implémentation ne doit pas
dépenser de tours à en ajouter une.

### Ce que je n'ai PAS pu mesurer, et ce que ça change

Les sondes de processus (`ps`, `stat`, `fuser`, `ss`) sont **refusées par la
politique d'outillage** de cette session de grooming. Trois conséquences, posées
honnêtement :

1. **La récidive `n=3` n'est pas ma mesure.** Elle est celle de l'orchestrateur,
   inscrite dans le corps du ticket le 2026-10-16 : `mitmdump` et
   `mika-pilot-egress-proxy --host-unix` démarrés le 2026-10-13 à 09:00Z,
   fichiers réinstallés le 2026-10-16 à 14:37Z. Je la cite, je ne la rejoue pas.
2. **La sémantique de `/proc/<pid>` n'est pas vérifiée.** Le mécanisme retenu
   (D2) repose sur le fait que le mtime du répertoire `/proc/<pid>` est l'instant
   de démarrage du processus. C'est vrai sous Linux, ce n'est pas documenté par
   `proc(5)`, et je n'ai pas pu le constater ici. Le contrat de vérification en
   fait une **question tranchée par V1** (contrôle négatif) sur l'hôte
   d'implémentation, avec un repli nommé.
3. **La disponibilité de `fuser` n'est pas vérifiée.** Le canary documente déjà
   sa possible absence (`:306-310`). Le verdict `undecidable` existe pour ce
   cas, et il ne refuse rien.

### Ce que ce vol porte, et pourquoi il s'arrête là

Le livrable de septembre était « les démons d'egress ne survivent plus à leur
code » — pluriel, deux lanceurs, un geste opérateur à étendre, et une couche
`Makefile`. Ce vol livre **le mécanisme et un seul câblage** : celui du
`mitmdump`. Trois raisons, dans l'ordre :

- **C'est le démon dont la péremption retire un contrôle de confinement.** Le
  relais périmé aveugle un journal (M5 du vol 1) ; le `mitmdump` périmé fait
  tourner la boucle sans l'exigence de sentinelle de mika#2724. Les deux comptent,
  l'un désarme.
- **C'est le seul des deux sans geste opérateur** (M4). Le relais a
  `--restart-relay` depuis mika#2049.
- **La borne par vol ne tient pas les deux.** § *Taille estimée*. Le second
  câblage est ~20 lignes sur un mécanisme déjà posé : un vol 3 court et
  mergeable seul, nommé en `Hors périmètre`.

La couche `Makefile` de septembre (« `make deploy` arrête les démons après
`install` ») **sort définitivement du périmètre, et pas par manque de place** :
avec D2, le premier dispatch qui suit un `install` relance lui-même le démon
périmé. Une couche qui tue des démons sur l'hôte de l'opérateur, hors de toute
fenêtre connue, serait un second mécanisme pour le même résultat — avec le
pouvoir de couper un pilote en vol que celui-ci n'a pas.

---

## Décisions de grooming

**D1 — L'action est la relance, jamais le refus.** Le ticket laisse le choix
(« relancé ou refusé de façon nommée »). Trois faits tranchent pour la relance :
M3 (tout deploy produira un verdict périmé, donc un refus arrêterait la boucle à
chaque déploiement), la posture déjà écrite du lanceur du helper (*fail-forward*,
`|| true` à `:2094`), et le fait qu'un refus demande un geste humain là où la
relance n'en demande aucun. **Ce vol n'ajoute aucun chemin de refus de
dispatch.**

**D2 — Le comparateur est l'ordre des mtimes, pas un condensé de contenu.**
La question « le code installé est-il plus récent que le démarrage du démon ? »
s'écrit en une ligne de shell : `[ "$fichier" -nt "/proc/$pid" ]`. Pas
d'époque à parser, pas de `CLK_TCK`, pas de format de `ps` à lire, aucun
nouveau binaire. **Le coût assumé, nommé :** par M3, un `make install` sans
changement de contenu produit un verdict périmé → **une** relance par démon et
par deploy. Ce faux positif est l'action souhaitée dans la quasi-totalité des cas
(un deploy est l'instant même où on veut que le démon reprenne son code) ; un
comparateur par condensé serait plus précis, demanderait un sceau écrit au
lancement, et serait **aveugle au seul démon qui compte aujourd'hui** — celui
qui tourne depuis le 13 octobre et n'a jamais rien scellé. Le raffinement part
en résidu dormant avec sa condition de réveil.

**D3 — La sémantique de `/proc/<pid>` est une hypothèse avec un repli nommé.**
Si V1 (contrôle négatif) ne peut pas être rendu vert sur l'hôte
d'implémentation, l'hypothèse est fausse et l'implémentation bascule sur
`/proc/<pid>/stat` champ 22 (`starttime`, en tics d'horloge) + `btime` de
`/proc/stat`, documentés tous deux par `proc(5)`, et **le dit dans le corps de
la PR**. Coût du repli : ~10 lignes, absorbées par la marge. Ce qui est interdit,
c'est de constater V1 rouge et de l'ajuster : V1 *est* le contrôle de
l'instrument.

**D4 — La relance est différée tant qu'un pilote frère vole, et le signal est le
budget d'inactivité du pilote lui-même.** Tuer le `mitmdump` sous un pilote qui
stream, c'est fabriquer la classe de panne que ce ticket existe pour éteindre.
Le signal : un `*.stderr` de `$_PILOT_LOG_DIR` modifié depuis **moins de 300 s**.
Ce nombre n'est pas inventé — c'est le budget d'inactivité de claude-pilot
(`idle_timeout`, watchdog cpp#170) : une session silencieuse plus longtemps est
déjà en train d'être tuée par son propre garde-fou. Le signal **sur-approxime**
« un frère vole », et c'est le bon sens de l'erreur : il diffère trop souvent,
jamais trop peu. La déférence est auto-guérissante — le prochain dispatch sans
frère en vol relance.

**D5 — Le verdict a quatre valeurs, et chacune a son compte.** `fresh` (silence,
comportement d'aujourd'hui), `stale` (relance), `stale_deferred` (D4),
`undecidable` (pas de porteur identifiable, `fuser` absent). Famille de jetons
`pilot_daemon_freshness.*`, calquée sur `pilot_egress_guard.*` : préfixe
`dispatch-lib: `, jeton contigu, comptable par `grep`. La doctrine maison est
explicite sur le motif — deux causes qui appellent deux gestes ne partagent pas
un jeton (`binary_missing` vs `unreachable`, `:1052-1057`).

**D6 — Si la relance ne libère pas le port, l'ancien démon est conservé.** Un
cinquième jeton, `stale_unstoppable` : le porteur a reçu un TERM, le port répond
toujours après la fenêtre d'attente, on **n'insiste pas** (pas de `KILL`) et le
lanceur retourne `0`. Un démon périmé qui sert vaut mieux qu'un port orphelin :
la posture héritée du helper laisserait partir le pilote sans chaîne Anthropic du
tout, c'est-à-dire exactement la panne de ce ticket.

**D7 — Chaque modification de `dispatch-lib.sh` passe par une édition en place.**
Consigne opérateur du 2026-10-15 (commentaire 19/21) : le vol précédent est mort
au tour 95 sur un `cp` vers ce chemin, refusé par le veto de destination
(`error_during_execution:after_deny`). **Aucun `cp`, `mv` ni redirection vers
`skills/bundled/_shared/dispatch-lib.sh` ni vers `test-dispatch-lib.sh`.** Outil
`Edit`, uniquement.

**D8 — La PR porte `Refs #2029`, pas `Closes #2029`.** Le vol 3 (câblage du
relais) reste. Même discipline que le vol 1 (D9 du plan précédent).

---

## Conception

Un seul fichier de code modifié, plus la suite de tests qui l'accompagne. Rien
dans le crate Rust, rien dans le `Makefile`, rien dans le canary.

### Livrable 1 — le mécanisme (`dispatch-lib.sh`, à côté des lanceurs)

Quatre fonctions et un vocabulaire, posés **avant** `_ensure_pilot_helper` pour
que le câblage les voie.

```bash
_PILOT_DAEMON_FRESH="fresh"
_PILOT_DAEMON_STALE="stale"
_PILOT_DAEMON_STALE_DEFERRED="stale_deferred"
_PILOT_DAEMON_UNDECIDABLE="undecidable"

# Le pid qui SERT le port, jamais un pid dont la ligne de commande ressemble.
# Même raison qu'au canary (:294-296) : `pgrep -f mitmdump` apparie aussi
# l'éditeur de l'opérateur, un grep, et ce script lui-même.
_pilot_daemon_holder_pid() {   # $1 = port TCP ; écrit le premier pid, ou rien
    command -v fuser >/dev/null 2>&1 || return 1
    ...  # fuser -n tcp "$1" 2>/dev/null, premier champ
}

# Vrai dès qu'un fichier de code existant est plus récent que le démarrage du
# processus. Un fichier absent est SAUTÉ, pas compté : l'addon GitHub est
# facultatif par construction (:1185-1187).
_pilot_daemon_code_is_newer() {   # $1 = pid, $2.. = fichiers
    ...  # [ "$f" -nt "/proc/$1" ]
}

# Un pilote frère vole-t-il ? (D4)
_pilot_sibling_pilot_in_flight() {
    ...  # find "$_PILOT_LOG_DIR" -maxdepth 1 -name '*.stderr' -newermt '-300 seconds'
}

# Le verdict, et la ligne nommée qui va avec. N'agit pas, ne refuse pas.
_pilot_daemon_freshness() {   # $1 = nom lisible, $2 = port, $3.. = fichiers
    ...  # écrit un des cinq jetons sur stdout, la ligne `dispatch-lib:` sur stderr
}
```

La ligne nommée, dans les trois cas qui parlent :

```
dispatch-lib: pilot_daemon_freshness.stale mitmdump (pid 2388646) a démarré avant l'installation de mika-pilot-github-auth-addon.py — relance (mika#2029)
dispatch-lib: pilot_daemon_freshness.stale_deferred mitmdump (pid 2388646) est périmé mais un pilote frère vole — relance différée au prochain dispatch (mika#2029)
dispatch-lib: pilot_daemon_freshness.undecidable mitmdump écoute sur 8892 mais son porteur n'est pas identifiable (fuser absent ou port sans porteur) — démon conservé (mika#2029)
```

`fresh` est **silencieux** : une ligne par dispatch sur l'état nominal noierait
les quatre autres, et le nominal est déjà ce que le code fait aujourd'hui.

### Livrable 2 — l'arrêt, par identité d'endpoint

```bash
_pilot_daemon_restart_by_port() {   # $1 = port, $2 = nom lisible
    # TERM au porteur, puis ATTENTE BORNÉE de la libération du port. Sans cette
    # attente, le lanceur ci-dessous rebind trop tôt, échoue sur « address in
    # use », et la relance laisse le trou qu'elle devait fermer.
    ...  # fuser -k -n tcp "$1" ; boucle de connect() bornée (~3 s)
    # Toujours tenu après la fenêtre -> `stale_unstoppable`, rc != 0 (D6).
}
```

Pas de `KILL` après le `TERM` : un `mitmdump` qui ignore un TERM est un état que
personne n'a mesuré, et l'escalade de signal se déciderait sur une mesure, pas
sur un plan.

### Livrable 3 — le câblage du chemin « already alive » de `_ensure_pilot_helper`

Le seul changement de comportement, à `:1188-1200`. La sonde de vivacité reste
le juge de « quelqu'un écoute » ; le verdict de fraîcheur décide seulement
**si l'on retourne** ou **si l'on retombe dans le lanceur déjà écrit**.

```bash
    if python3 -c "… connect(('127.0.0.1', $_PILOT_HELPER_PORT)) …" 2>/dev/null; then
        case "$(_pilot_daemon_freshness mitmdump "$_PILOT_HELPER_PORT" \
                    "$_PILOT_HELPER_ADDON" "$_PILOT_GH_HELPER_ADDON" "$_PILOT_HELPER_BIN")" in
            "$_PILOT_DAEMON_STALE")
                # Pas de `return` : on retombe dans le lancement + la sonde de
                # bind bornée (:1209-1237), inchangés. Un helper qui ne revient
                # pas produit donc la ligne `failed to bind` existante et la
                # POSTURE HÉRITÉE du site d'appel (`|| true`, :2094) : le
                # dispatch continue. C'est la même posture qu'un helper
                # simplement mort — ce vol n'en invente pas une.
                _pilot_daemon_restart_by_port "$_PILOT_HELPER_PORT" mitmdump || return 0
                ;;
            *) return 0 ;;
        esac
    fi
```

Les trois chemins d'erreur qui précèdent (`:1172-1187` — binaire absent, addon
Anthropic absent, addon GitHub absent) sont **intacts** : le verdict de
fraîcheur ne se pose qu'après eux, donc il n'est jamais interrogé sur un fichier
dont l'absence a déjà été nommée.

---

## Étapes d'implémentation

1. Lire `dispatch-lib.sh:1052-1244` d'un bloc avant d'écrire : le vocabulaire des
   motifs de mika#2049 et les deux lanceurs sont le contexte complet du
   changement.
2. Livrable 1, par éditions en place (D7), inséré juste avant
   `_ensure_pilot_helper`.
3. Livrable 2, à la suite du livrable 1.
4. Livrable 3 : la `case` dans le chemin « already alive ».
5. Tests, sur le patron de `_egress_guard_probe` (M7) : **V1 d'abord** (le
   contrôle négatif tranche D3), puis V2, V3, V4, V5.
6. Constater V2 **rouge** contre `main` avant le correctif (contre-vacuité,
   V5), et le consigner dans le corps de la PR.
7. `make test-dispatch-lib`, `make lint`, `make verify-bundled-skills`,
   `cargo test`, `scripts/verify-pipeline.sh origin/main`.
8. Deux paragraphes dans `docs/operator/pilot-egress-relay.md` : les cinq jetons
   et ce que chacun demande à l'opérateur (`stale_deferred` et `undecidable`
   n'en demandent **aucun** ; `stale_unstoppable` demande de regarder qui tient
   le port 8892). Hors budget de lignes, dans le budget de tours.
9. PR : `Refs #2029`, la mesure de M3 (le faux positif assumé), le repli D3 s'il
   a été pris, et le vol 3 nommé.

---

## Contrat de vérification

Toutes les sondes vivent dans `skills/bundled/_shared/test-dispatch-lib.sh`
(`make test-dispatch-lib`), sur un fixture unique à états — un faux démon qui
écoute vraiment le port, un faux répertoire de code, `PILOT_LOG_DIR` redirigé.
Aucune sonde ne touche `/var/log`, ni le port 8892 réel, ni le registre de
sentinelles de l'opérateur.

| sonde | état | attendu |
|---|---|---|
| **V1 — contrôle négatif** | fichiers de code posés **avant** le lancement du faux démon | verdict `fresh`, **aucun** TERM, aucune ligne `pilot_daemon_freshness.`, le lanceur retourne `0` |
| **V2 — contrôle positif** | même état, puis `touch` d'un addon | verdict `stale`, ligne `pilot_daemon_freshness.stale`, le porteur est mort, un nouveau démon écoute |
| **V3 — déférence** | état de V2 + un `*.stderr` frais dans `PILOT_LOG_DIR` | verdict `stale_deferred`, **aucun** TERM, le démon d'origine écoute encore, rc `0` |
| **V4 — indécidable** | port tenu par un porteur que la découverte ne rend pas (`fuser` neutralisé dans le sous-shell) | verdict `undecidable`, aucun TERM, rc `0` |
| **V5 — contre-vacuité** | V2 rejouée contre `main` | **rouge** : le chemin « already alive » y retourne `0` en silence. À constater avant le correctif et à consigner dans la PR. |

V1 est double : contrôle négatif du détecteur **et** contrôle de l'instrument de
D3. Un `/proc/<pid>` dont le mtime ne serait pas l'instant de démarrage rendrait
V1 indécidable ou rouge — c'est le signal de bascule vers le repli nommé, et la
raison pour laquelle V1 s'écrit en premier.

---

## Risques nommés

**R1 — La relance casse un pilote frère que D4 n'a pas vu.** Le signal de D4 est
une sur-approximation, mais une session dont le `.stderr` est silencieux depuis
plus de 300 s et qui *n'est pas* en train d'être tuée serait un angle mort. Coût :
une requête Anthropic échouée chez le frère, suivie d'une relance immédiate du
démon sur le même port — le SDK retente. **Ce qui rend ce risque acceptable, et
qu'il faut écrire dans la PR :** le régime alternatif est une boucle entière qui
tourne sans l'exigence de sentinelle de mika#2724.

**R2 — Un faux positif par deploy (M3, assumé en D2).** Borne : une relance par
démon et par `make install`, entre deux dispatches, différée tant qu'un frère
vole. Si cette borne est dépassée en mesure, le résidu dormant « comparateur par
condensé » se réveille.

**R3 — `fuser` sur un port peut rendre plusieurs pids.** Le livrable 1 prend le
premier ; le livrable 2 envoie le TERM via `fuser -k`, qui les signale tous.
L'implémentation doit vérifier que la ligne nommée reste lisible quand `fuser`
en rend deux (un `mitmdump` qui a forké), et ne doit pas construire un pid à
partir d'une concaténation.

**R4 — La suite de tests tourne sur l'hôte de l'opérateur.** Le fixture doit
rediriger `PILOT_LOG_DIR`, choisir un port éphémère (jamais 8892), et ne jamais
appeler `fuser -k` sur un port qu'il n'a pas lui-même ouvert. Le précédent
existe et il a mordu : `_stage_pilot_egress_sentinel` a déposé deux entrées
vivantes dans le registre réel de l'opérateur au premier essai de mika#2716
(`:1626-1631`).

**R5 — Le `case` du livrable 3 est sur le chemin de chaque dispatch.** Son coût
est un `fuser` et jusqu'à trois `test -nt` quand le démon est vivant. Si
l'implémentation mesure un coût non négligeable, elle le dit dans la PR plutôt
que de déplacer la sonde ailleurs.

---

## Hors périmètre

- **Le câblage du relais d'egress (`_ensure_pilot_egress_proxy:1319-1321`) —
  vol 3.** Même mécanisme, un `case` de ~20 lignes, plus la variante
  « porteur d'une socket unix » de la découverte de pid (`fuser "$sock"`, la
  forme déjà employée par le canary) et l'extension de
  `--restart-relay` au port 8892. La posture y est *fail-closed* : une relance
  qui ne rebind pas **refuse** le dispatch par la machinerie mika#2049 déjà
  écrite — un comportement différent, qui mérite ses propres sondes.
  **Tracked in:** senara-solutions/mika#2029 (ce ticket reste ouvert après ce
  vol — D8).
- **La couche `Makefile`** de septembre (« `make deploy` arrête les démons après
  `install` ») — retirée du périmètre du ticket, pas reportée. Motif écrit en
  § *Contexte*.
- **Élargir la posture du helper vers un refus de dispatch.** Un `mitmdump`
  absent ou mort laisse aujourd'hui partir le pilote (`|| true`, `:2094`). C'est
  discutable, et ça n'appartient pas à ce ticket : le propriétaire est la lignée
  mika#2716 / mika#2724. Résidu dormant ci-dessous.
- **La cause du stall** — cpp#177, fermée. Rien à porter.
- **Le texte des callbacks** — livré par le vol 1 (#2720).

---

## Résidus dormants, avec leur condition de réveil

- **Le comparateur par condensé de contenu.** Un sceau écrit au lancement
  (pid + condensé des fichiers de code) rendrait le verdict exact et supprimerait
  le faux positif de M3. **Réveil :** une mesure montrant que les relances par
  deploy coûtent quelque chose — une session frère cassée malgré D4, ou plus
  d'une relance par deploy et par démon.
- **La posture du helper.** Un pilote qui part sans chaîne Anthropic ne peut rien
  produire ; la posture *fail-forward* le laisse partir. **Réveil :** une session
  tuée sans production dont le journal d'egress montre un `mitmdump` absent au
  moment du lancement — c'est-à-dire la classe de ce ticket, par une autre porte.
- **L'origine du TTFT bimodal** (≤ 29 s ou 300–389 s sur 371 appels, septembre).
  cpp#177 a rendu la fenêtre d'attente robuste à ce délai sans l'expliquer.
  **Réveil :** `idle_timeout` au-dessus de 2 occurrences par semaine, ou un kill
  `stream_stalled` sur une session dont le TTFT dépasse 300 s.
- **La classe « mort-née »** (session initialisée, zéro événement de flux) : n=3,
  toutes le 2026-09-03. **Réveil :** une occurrence de `(0 content stream events
  this session)` — motif **ancré sur la parenthèse**.

---

## Definition of Done

- Les quatre fonctions du livrable 1 et celle du livrable 2 existent dans
  `dispatch-lib.sh`, et le `case` du livrable 3 est posé dans le chemin
  « already alive » de `_ensure_pilot_helper`.
- Les cinq jetons `pilot_daemon_freshness.*` sont émis avec le préfixe
  `dispatch-lib: ` et un jeton contigu, comptables par `grep -E '^dispatch-lib:
  pilot_daemon_freshness\.'`.
- Les trois chemins d'erreur de `_ensure_pilot_helper` (`:1172-1187`) et la
  posture `|| true` du site d'appel (`:2094`) sont **inchangés**.
- V1–V4 passent ; V2 a été **vue rouge** contre `main` (V5) et c'est écrit dans
  la PR.
- Le verdict `fresh` n'écrit rien.
- `make test-dispatch-lib`, `make lint`, `make verify-bundled-skills`,
  `cargo test` et `scripts/verify-pipeline.sh origin/main` passent.
- `docs/operator/pilot-egress-relay.md` nomme les cinq jetons et le geste (ou
  l'absence de geste) que chacun demande.
- Si le repli D3 a été pris, la PR le dit et nomme ce que V1 a montré.
- La PR porte `Refs #2029` (**pas** `Closes`), nomme le vol 3, et a été produite
  sans aucun `cp`/`mv`/redirection vers `dispatch-lib.sh` (D7).

## Acceptance criteria

Le corps de mika#2029 ne porte pas de section `## Acceptance criteria` : il porte
un « **Attendu du vol** » (vol 2, 2026-10-16) et renvoie aux AC de #2035 comme
référence. Les critères ci-dessous transcrivent l'attendu et bornent #2035 au
périmètre de ce vol.

1. **Un démon dont le code installé est plus récent que son démarrage est
   détecté au dispatch suivant.** Le chemin « already alive » de
   `_ensure_pilot_helper` rend un verdict au lieu de retourner `0` en silence.
   *(Attendu du vol, phrase 1.)*
2. **Il est ensuite relancé, ou son non-traitement est nommé.** `stale` →
   relance ; `stale_deferred`, `undecidable`, `stale_unstoppable` → démon
   conservé et **une ligne `dispatch-lib:` qui le dit**. *(Attendu du vol,
   phrase 1 ; D1 tranche « relancé ou refusé » en faveur de la relance.)*
3. **Un test fixe ce comportement avec un contrôle positif et un contrôle
   négatif.** V2 (fichier plus récent → détection) et V1 (démon plus récent que
   son code → aucune détection, aucun TERM). *(Attendu du vol, phrase 2.)*
4. **Aucun dispatch n'est refusé par ce vol.** Aucun nouveau chemin de refus ;
   la posture du site d'appel est inchangée.
5. **Aucune relance ne part sous un pilote frère en vol.** V3.
6. **Le périmètre non livré est nommé, pas tu.** Le câblage du relais et
   l'extension de `--restart-relay` portent une ligne `Tracked in:`, et le
   ticket reste ouvert.

---

## Fire-Disposition

Ce vol livre **un** détecteur au sens de mika#2306 : `_pilot_daemon_freshness`,
dont le chemin de succès est « aucune violation trouvée » (verdict `fresh`,
silence). Les sondes V1–V4 interrogent le comportement de ce détecteur ; V5 est
un constat de contre-vacuité. La section est donc requise.

**Option retenue : (a) exception nommée en allowlist — l'allowlist est VIDE, et
l'exception nommée est le motif `stale_deferred`.**

La population en violation au moment de l'atterrissage est connue et mesurée :
le `mitmdump` de l'hôte de dispatch, démarré le 2026-10-13, portant des addons
réinstallés le 2026-10-16 (corps du ticket). Elle n'est **pas exceptée, elle est
réparée** — par l'action du détecteur lui-même, au premier dispatch. Écrire une
entrée d'allowlist la nommant serait écrire un instrument qui mentirait : l'entrée
désignerait un pid qui n'existe plus dès que la garde a tourné une fois, et
l'assertion auto-nettoyante que (a) exige la rendrait rouge immédiatement.

L'exception que ce vol porte réellement est **`stale_deferred`**, et elle a les
trois propriétés que (a) demande :

- **Elle nomme la donnée précise** : un `*.stderr` de `$_PILOT_LOG_DIR` modifié
  depuis moins de 300 s — pas « un pilote tourne peut-être », une borne et un
  fichier.
- **Elle référence un suivi** : la ligne porte `mika#2029`, qui reste ouvert
  après ce vol.
- **Elle est auto-nettoyante par construction** : aucune main ne la retire. Le
  prochain dispatch sans frère en vol relance le démon et le motif disparaît. Une
  exception qui devient stale ici ne peut pas rester silencieuse — elle se
  transforme en `stale` au dispatch suivant.

L'assertion auto-nettoyante du détecteur lui-même est **V1**, et c'est la même
sonde que le contrôle négatif du ticket : elle affirme qu'un démon plus récent
que son code lit `fresh` et **n'est pas relancé**. Elle rougit dans la direction
dangereuse — un prédicat qui déraperait vers « toujours périmé » relancerait le
`mitmdump` à chaque dispatch, coupant les tunnels en vol, c'est-à-dire la panne
que ce ticket éteint. Un détecteur de péremption sans ce contrôle est un
redémarreur permanent déguisé en garde.

Le détecteur atterrit donc **armé**, avec la seule violation connue réparée par
sa propre action, et sans dette à ouvrir.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `dispatch-lib.sh` — vocabulaire des cinq jetons + `_pilot_daemon_holder_pid` + `_pilot_daemon_code_is_newer` + `_pilot_sibling_pilot_in_flight` + `_pilot_daemon_freshness` (verdict et lignes nommées), commentaires D2/D3/D4 inclus | 76 |
| `dispatch-lib.sh` — `_pilot_daemon_restart_by_port` (TERM par identité d'endpoint, attente bornée de la libération, `stale_unstoppable`) | 18 |
| `dispatch-lib.sh` — câblage du chemin « already alive » de `_ensure_pilot_helper` (`:1188-1200`) + commentaire sur la posture héritée | 18 |
| tests (`test-dispatch-lib.sh`) — fixture à états sur le patron de `_egress_guard_probe`, puis V1–V4 | 72 |

Total estimé : 184 lignes

Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Pourquoi un seul vol tient ici.** 184 contre 243 : la marge est de 24 %, et
elle absorbe le facteur de sous-estimation bas de la distribution mika#2693
(1,3 × 184 = 239, sous la borne). Le vol 1 s'était tenu au même étalon (174 →
226) et avait sorti ce livrable pour cette raison exacte. Trois propriétés
rendent cette estimation moins fragile que les trois morts du 2026-10-14 : un
seul fichier de code modifié, aucun fichier créé, aucun nouveau harnais de test
(le fixture est un patron existant, M7), et **chaque site d'édition est donné au
`file:line`**. Le repli D3, s'il est pris, coûte ~10 lignes et tient dans la
marge.

Ce qui a été sorti pour tenir la borne est nommé en § *Hors périmètre* avec sa
ligne `Tracked in:` : le câblage du relais, sa variante de découverte par socket
unix, et l'extension de `--restart-relay`. Ce vol 3 est mergeable seul parce que
le mécanisme qu'il consomme atterrit ici.
