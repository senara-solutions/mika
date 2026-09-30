---
module: mika-spirit, build, observability
tags: [wedge, deadlock, symbolication, diagnosability, observability, rt003, ci-guard]
problem_type: observability-gap
category: chore
issue: mika#1719
type: chore
date: 2026-09-30
supersedes: docs/plans/2026-09-10-003-chore-1719-spirit-wedge-diagnosability-symbols-plan.md
---

# mika#1719 — rendre la prochaine occurrence diagnosticable : symboles au binaire déployé

> **Ce plan remplace celui du 2026-09-10** (`2026-09-10-003-…`, architecte READY
> première passe, lisible au commit `5ad1e7e1`). Son cadrage est repris à
> l'identique — il était juste. Ce qui change est **exécutable par qui** : trois
> de ses huit étapes ne sont pas à la portée d'un pilote dispatché, et l'une
> d'elles a tué une session de façon mesurée (§3). S'ajoute un détecteur (§5),
> parce que le réglage livré est une ligne dont le retour serait silencieux.

## 1. Le reliquat, et ce que le re-groom du 2026-09-30 a re-mesuré

Le corps de mika#1719 (2026-07-02) décrit un wedge process-wide de `mika-spirit` :
29 threads sur 30 parkés au même frame, 8 h 30 de panne silencieuse, cinq
récurrences en trois jours (n=1 → n=5). Il propose cinq remèdes durables.
**Quatre sur cinq sont livrés.**

| Remède du corps | État | Preuve re-mesurée le 2026-09-30 |
|---|---|---|
| 2. Auditer les sites de détention de verrou | **livré** | mika#1723, PR#1726 mergée le 2026-07-06 (`3f87e9a9`) — `handlers.rs` : `Ref` DashMap détenu par le scrutinee à travers un `.insert()` de même shard |
| 3. Interdire structurellement la forme | **livré** | `Cargo.toml:216` → `significant_drop_in_scrutinee = "warn"` ; la liste de types du footgun vit dans `clippy.toml:12` |
| 4. Détecter le wedge au lieu de le subir 8 h | **livré autrement** | cm#126 — `control-monitor/scripts/host-probes.sh:86-90`, heartbeat inversé sur `localhost:8081/healthz`, cron */10 |
| 5. Corriger la double-écriture des logs | **livré** | mika#2195 / PR#2210, clos le 2026-09-06 |
| 1. Reconstruire avec des symboles | **NON FAIT — seul reliquat** | `Cargo.toml:203` → `strip = true`, dans un `[profile.release]` ouvert ligne 200 |

**Quatre références de ligne du plan du 10/09 ont bougé et sont corrigées ici** —
`strip` était annoncé à `Cargo.toml:169`, il est à `203` ; le lint à `181-182`,
il est à `216` ; RT#003 à `liveness.rs:156-158`, il est à `155-157` ;
`spawn_engine_wedge_watchdog` reste à `liveness.rs:160`. Un plan qui cite une
ligne fausse envoie son implémenteur lire le mauvais endroit, et c'est la
première chose qu'un re-groom doit refaire.

**Ce qui n'a pas pu être re-mesuré depuis ce worktree, et qui est donc porté
comme à vérifier plutôt que comme un fait** : la taille du binaire déployé
(référence datée du 10/09 : 34 Mo strippé) et la sortie de `nm` sur
`~/.local/bin/mika-spirit` (`no symbols` au 10/09). Le bac à sable de dispatch
refuse les lectures d'outils hors de son empreinte — une tentative de
`command -v addr2line` le 2026-09-30 a rendu
`no matching policy rule -- denied by default`. Ces deux valeurs sont des
mesures **d'implémentation**, pas de grooming : l'étape 1 du §8 les relève.

## 2. Pourquoi ce reliquat vaut encore quelque chose

Ce n'est pas de l'hygiène de build. C'est le facteur qui a transformé un bug
d'une ligne en trois jours d'enquête et cinq pannes.

Le diagnostic de juillet a dû procéder par soustraction d'adresses : cinq
captures gdb rendant `?? ()` sur chaque frame, une base PIE recalculée à la
main, trois hypothèses successivement falsifiées (`AgentState.skills` —
falsifiée ; double-init du souscripteur tracing — falsifiée ;
`flush_failed_sends` — non concluante), et un tableau d'invariants lui-même
révisé à n=4 (le « ≥1 thread », après qu'une croissance en 4× avait été lue sur
deux points de mesure). Le site fautif n'a été nommé qu'au quatrième jour. Un
`addr2line` sur un binaire porteur de tables de lignes l'aurait rendu au premier.

mika#1722 a déjà fermé la **moitié amont** de cette chaîne :
`scripts/mika-spirit-wedge-capture.sh` (présent, 5 634 o) capture
`/proc/<pid>/maps` au même instant que la pile, donc la base PIE est un artefact
indépendant et non une valeur dérivée dans l'analyse. La **moitié aval** — offset
→ nom de fonction — reste ouverte, et l'en-tête du script le dit de lui-même :
la corroboration passe aujourd'hui par un `addr2line` sur *une reconstruction*,
ce qui est « necessarily indirect — both endpoints of the subtraction are the
thing being verified ».

Formulé en termes de risque : la classe de bug précise est morte (#1723 +
#1724), mais **la classe d'enquête ne l'est pas**. Tout futur park process-wide —
sur cet hôte ou chez un tenant cloud, dont l'image hérite du même profil
(`Dockerfile.agent:46` bâtit `--release` sans profil propre) — repart d'un
binaire muet.

## 3. Ce que le plan du 10/09 prescrivait, et qu'un pilote ne peut pas faire

C'est le livrable principal de ce re-groom, et il ne vient pas d'un raisonnement
mais d'une mesure : **le pilote dispatché le 2026-09-10 est mort en exécutant
l'étape 3 de ce plan.** Session `4667a1c2`, 13 tours, 36 min, zéro commit
au-delà du groom. Cinq denies, dont le cinquième létal :

```
[2026-09-10T19:16:52.386Z] [policy:deny] Bash: /usr/bin/time -v cargo build
  --release --features telemetry --bin mika-spirit > /data/workspace/…
[2026-09-10T19:16:52.390Z] [error] error_during_execution:after_deny
```

Trois prescriptions sont hors de portée, et chacune appelle une reformulation
différente — les confondre reproduirait la mort :

| Prescription du 10/09 | Pourquoi inexécutable | Reformulation |
|---|---|---|
| Étape 3 — « mesurer la **durée** du build » | La forme employée (`/usr/bin/time -v … > fichier`) est hors allow-list et son deny est **terminal** (`after_deny`, classe cpp#128) | **Retirée du périmètre du pilote**, avec un motif de fond — voir ci-dessous |
| Étape 5 — `make deploy` | `deploy` = `deploy-info build-dashboard build install restart check-webhook-chain` : installe dans `~/.local/bin` **et redémarre les services**. Hors de l'autorité d'un pilote, et RT#003 place la manœuvre de service chez l'opérateur | Devient la **sonde opérateur** du §9 |
| Étape 5 — `mika-spirit-wedge-capture.sh $(pidof mika-spirit)` | Le script émet dans `/tmp/spirit-wedge-<ts>/`, où toute écriture est refusée (mika#2211/#2548) ; et `pidof mika-spirit` avait déjà produit un deny dans la même session | Devient la **sonde opérateur** du §9 |

**Retirer la durée n'ampute aucune décision, et c'est pourquoi c'est la bonne
reformulation plutôt qu'une concession au bac à sable.** `line-tables-only`
ajoute l'émission des tables de lignes ; le temps dominant d'un release qui
porte `lto = true` et `codegen-units = 1` est le LTO et le codegen, tous deux
inchangés. Et surtout, le seuil qui décide du repli (§4) est une **taille**, pas
une durée : aucune valeur de durée ne change la conduite. Une mesure qui ne
décide de rien et qui tue la session qui la prend n'est pas une mesure, c'est un
piège.

**Le fait qui débloque le reste, et il est petit** : `make install`
(`Makefile:32-40`) fait `cp target/release/$bin $(INSTALL_DIR)/$bin.tmp` puis
`mv`. Le binaire déployé est donc **la copie** de celui que le pilote bâtit, pas
un rebuild. La symbolisation peut être prouvée dans le worktree, sur
`target/release/mika-spirit`, sans déploiement et sans capture gdb — et cette
preuve porte sur le binaire qui sera servi, parce que `cp` ne change pas un octet.

## 4. Approche retenue

Modifier `[profile.release]` dans `Cargo.toml`, en gardant `lto` et
`codegen-units` inchangés :

```toml
[profile.release]
lto = true
codegen-units = 1
strip = "none"              # était : true
debug = "line-tables-only"  # nouveau
```

**Les deux lignes, et chacune est inutile sans l'autre.** C'est le point que le
détecteur du §5 existe pour tenir : `strip = "none"` seul ne donne rien à
conserver, le profil release valant `debug = 0` par défaut, donc `addr2line`
rendrait `??:0` ; et `debug = "line-tables-only"` seul fait émettre des tables
que `strip` retire ensuite. Chacune des deux, lue isolément, a l'air de
suffire — et un futur éditeur qui en retire une aura raison de croire que
l'autre porte le réglage.

`line-tables-only` plutôt que `strip = "debuginfo"` (qui ne garderait que
`.symtab` : des noms, pas de `fichier:ligne`) et plutôt que `debug = true`
(DWARF complet, plusieurs centaines de Mo). Il donne exactement ce que le
diagnostic de juillet réclamait — un nom **et** un `fichier:ligne` — pour un
surcoût borné.

**Profil unique et global, sans `release-with-debug` séparé**, pour une raison de
fond : un binaire diagnosticable ne sert que s'il est *celui qui a wedgé*. Un
profil parallèle produit un binaire qu'on n'exécute pas, et laisse les tenants
cloud sur l'impasse de juillet.

**Repli, pris seulement si la mesure le commande** : `split-debuginfo = "packed"`,
qui laisse le binaire déployé mince et archive un `.dwp` à côté. Il a un coût à
nommer : le `.dwp` doit alors accompagner le binaire jusqu'à la machine qui
symbolise, ce qui rend l'artefact déployé non autosuffisant. D'où le seuil
généreux de l'AC3 — le repli ne doit pas se déclencher sur le régime nominal.

## 5. Le détecteur, et pourquoi il faut en livrer un

`strip = true` était **une ligne**, et son retour serait silencieux. Un futur
éditeur qui veut alléger les images Docker la remet, aucun test ne rougit, le
binaire redevient muet — et on l'apprend au prochain wedge, c'est-à-dire au
moment le plus coûteux possible. C'est la classe mika#2205 : *un réglage
silencieusement annulé se lit exactement comme un réglage en vigueur.* Livrer le
réglage sans sa garde, c'est le livrer pour la durée de la mémoire de celui qui
l'a posé.

`scripts/check-release-debuginfo.sh`, job CI `release-debuginfo-lint`, cible
`make check-release-debuginfo` — le motif des quinze `check-*.sh` du dépôt, dont
`check-pilot-turn-ceiling-labels.sh` est le plus récent exemple.

**Le prédicat est positif et borné**, et les deux adjectifs sont porteurs :

1. **Borné au bloc.** Le scan extrait le bloc `[profile.release]` du `Cargo.toml`
   du workspace, de son en-tête jusqu'à la prochaine ligne commençant par `[`.
   Sans cette borne, un `strip = true` légitime dans un `[profile.bench]` plus
   bas ferait rougir un arbre propre, et une garde qui crie à tort finit muselée.
2. **Anti-vacuité.** Bloc absent, vide, ou fichier illisible ⇒ **exit 3**, jamais
   exit 0. Un renommage ou un déplacement du profil rendrait sinon le scan
   silencieusement inerte, et *un scan aveugle se lit exactement comme un arbre
   propre*.
3. **Terme A — `strip`.** Absent, ou `none` (nu, `"none"`, `'none'`). Toute autre
   valeur refuse. Le terme est **positif** et non un test d'égalité à `true` :
   `strip = "symbols"` est équivalent à `true` et passerait une denylist.
4. **Terme B — `debug`.** Présent **et** portant une valeur qui conserve les
   tables de lignes : `"line-tables-only"`, `1`, `2`, `true`, `"limited"`,
   `"full"`. Absent, `0`, `false`, `"none"` refusent. C'est ce terme qui attrape
   la moitié du §4 qu'un éditeur croirait redondante.
5. **Chaque refus nomme son remède** — la ligne à écrire, pas seulement la ligne
   fautive. Un refus qui ne nomme pas sa levée est un refus qu'on contourne au
   jugé.

**Contrôles négatifs, six fixtures, dont au moins trois vues rouges avant de
conclure** (motif mika#2496) :

| # | fixture | attendu | ce qu'elle atteste |
|---|---|---|---|
| N1 | `strip = true` + `debug = "line-tables-only"` | ROUGE | le terme A mord |
| N2 | `strip = "none"`, **aucune** ligne `debug` | ROUGE | le terme B mord — le piège du prédicat naïf |
| N3 | `strip = "none"` + `debug = 0` | ROUGE | le terme B est positif, pas une denylist de `false` |
| N4 | `strip = "symbols"` + `debug = "line-tables-only"` | ROUGE | le terme A n'est pas un test d'égalité à `true` |
| N5 | bloc conforme **+** un `[profile.bench]` portant `strip = true` | **VERT** | le bloc est borné ; sans cette fixture, « le scan lit le bon bloc » est indistinguable de « le scan rougit sur tout `strip = true` » |
| N6 | `[profile.release]` absent du fichier | **exit 3** | l'anti-vacuité ; sans elle, un scan devenu aveugle rend 0 |

N5 et N6 sont les deux qui comptent le plus : ce sont les seules qui distinguent
une garde qui *regarde quelque chose* d'une garde qui passe par accident.

Les fixtures vivent sous `.pilot-scratch/` pendant le développement et sous
`scripts/fixtures/release-debuginfo/` dans le livrable, pour que le contrôle
négatif soit rejouable par le suivant et non seulement par celui qui l'a écrit.

## Fire-Disposition

Le plan livre un détecteur (§5), donc cette section est requise (mika#2306).

**Option (a) — exception nommée en allowlist, allowlist livrée VIDE.**

La population de violations est **vide par construction** : le même commit qui
arme la garde pose la valeur conforme, donc il n'existe aucune violation
préexistante à exempter. L'allowlist (`scripts/release-debuginfo-allowlist.txt`)
est créée vide et **pinnée vide** par une assertion frère dans le script, sur le
motif de `scripts/pilot-push-allowlist.txt` (mika#2520) et des scans à allowlist
vide de mika#2496.

**Assertion auto-nettoyante.** Chaque entrée éventuelle doit nommer (i) le
fichier et la clé précise, (ii) un ticket de suivi, (iii) sa raison. Le scan
compare l'allowlist **dans les deux sens** : une entrée qui ne correspond plus à
aucune violation réelle fait **rougir le build**, le jour de la réparation et non
des mois après. Sans cette seconde direction, une exemption survit à son motif et
exempte silencieusement un futur homonyme.

**Doctrine de résolution, écrite dans l'en-tête du script** : quand la garde
tire, on **répare le profil** ; on n'ajoute pas de ligne à l'allowlist
(mika#2201). Un profil qu'on ne veut pas rendre diagnosticable est un profil
dont il faut discuter, pas un profil à exempter.

Le détecteur est donc livré **armé**, et ce n'est pas de l'imprudence : le coût
d'un faux positif est un build rouge sur une ligne de configuration, visible et
réparable en un caractère ; le coût d'un faux négatif est le retour silencieux de
la panne que ce ticket entier existe pour raccourcir. L'asymétrie penche du côté
armé. Précédent et argument : mika#2272, *« zéro était l'absence de mesure, pas
la présence de prudence »*.

## 6. Definition of Done

- **DoD1** — `[profile.release]` du `Cargo.toml` du workspace porte
  `strip = "none"` **et** `debug = "line-tables-only"` ; `lto = true` et
  `codegen-units = 1` sont inchangés.
- **DoD2** — Sur `target/release/mika-spirit` bâti depuis ce profil, un offset de
  symbole obtenu par `nm` est rendu par `addr2line -f -C -e` en un nom **et** un
  `fichier:ligne`. La preuve est la sortie de commande, pas la lecture du
  `Cargo.toml`.
- **DoD3** — La taille du binaire est relevée avant/après et reportée dans le
  corps de PR. Au-delà de **3×** la référence, le repli
  `split-debuginfo = "packed"` est appliqué et la mesure refaite.
- **DoD4** — `scripts/check-release-debuginfo.sh` existe, est armé en CI sous
  `release-debuginfo-lint`, passe sur l'arbre conforme, et ses six fixtures de
  contrôle négatif rendent le verdict du tableau du §5.
- **DoD5** — Une entrée `docs/solutions/` porte la procédure de symbolisation
  d'un wedge, avec la sortie réelle du DoD2 comme exemple travaillé.
- **DoD6** — Aucun fichier `.rs` n'est modifié.

## 7. Acceptance criteria

**AC1 — Une adresse redevient un nom et une ligne, sur le binaire qui sera
servi.** Sur `target/release/mika-spirit` bâti par `make build` dans le
worktree : prendre l'offset d'une fonction connue via `nm`, le passer à
`addr2line -f -C -e target/release/mika-spirit <offset>`, obtenir un symbole et
un `fichier:ligne`. Cette preuve porte sur le binaire déployé parce que
`make install` le copie sans le reconstruire (§3).

> **Ne pas confondre « pas de symbole » avec « symbole mangelé ».** Si la sortie
> est `_RNvNtCs…` plutôt qu'un nom lisible, la chaîne offset → symbole → ligne
> **a fonctionné** : le démanglage v0 de Rust n'est pas toujours géré par le
> `-C` de binutils, et `rustfilt` le complète. Un implémenteur qui lit un nom
> mangelé comme un échec conclurait à l'inverse de la mesure. Si la binutils est
> absente de l'hôte, le repli nommé est `llvm-addr2line` (composant rustup
> `llvm-tools`) — l'outil change, l'AC ne change pas.

**AC2 — Contrôle négatif de la symbolisation, dans la même session qu'AC1.** Un
`addr2line` sur une adresse hors du segment de code échoue lisiblement (`??`,
`??:0`) et non silencieusement en rendant un symbole voisin plausible. Une sonde
a besoin de ses deux contrôles au même moment : sans celui-ci, « symbolisé » et
« symbolisé n'importe comment » rendent des octets identiques.

**AC3 — Le surcoût est chiffré et borné.** Taille avant/après dans le corps de
PR. Référence à re-mesurer (le 10/09 donnait 34 Mo strippé ; le §1 dit pourquoi
elle n'est pas re-mesurable depuis un worktree de dispatch). Seuil de repli : 3×.

**AC4 — Les propriétés d'exécution ne bougent pas.** `lto` et `codegen-units`
intacts, aucun `.rs` touché. Vérifiable par lecture du diff.

**AC5 — La garde regarde quelque chose.** `make check-release-debuginfo` rend 0
sur l'arbre livré, et les six fixtures du §5 rendent leur verdict attendu — les
trois au moins **vues rouges** avant de conclure, N5 vue verte, N6 vue en exit 3.

**AC6 — Le suivant sait quoi faire à 3 h du matin.** L'entrée `docs/solutions/`
donne la chaîne complète : capture → base PIE depuis `maps` → offset →
`addr2line`. Elle porte l'exemple travaillé de l'AC1 et **nomme l'étape capture
comme geste opérateur** avec le pointeur vers
`scripts/mika-spirit-wedge-capture.sh` — sans prétendre porter une sortie gdb
réelle, que le pilote ne peut pas produire (§3). C'est ce document, pas ce plan,
qui sera lu à la prochaine occurrence.

## 8. Étapes

1. Relever l'état de départ, **sans redirection ni chronométrage** (§3) : taille
   de `target/release/mika-spirit` après un `make build` de référence, et sortie
   de `nm` sur ce binaire (attendu : aucun symbole).
2. Appliquer les deux lignes du §4 à `[profile.release]`.
3. `make build`. Relever la nouvelle taille. **Ne pas chronométrer.**
4. Si le seuil de l'AC3 est franchi, appliquer `split-debuginfo = "packed"` et
   refaire la mesure **avant** d'aller plus loin.
5. Contrôle positif (AC1) et contrôle négatif (AC2) dans la même session.
6. Écrire `scripts/check-release-debuginfo.sh` (§5), son allowlist vide, sa cible
   `make` et son job CI. Voir chacune des six fixtures rendre son verdict.
7. Écrire l'entrée `docs/solutions/` (AC6) avec la sortie réelle de l'étape 5.
8. Reporter dans le corps de PR : les deux tailles, le verdict de repli, la
   sortie de l'AC1 et celle de l'AC2.

**Le ticket compagnon côté `control-monitor`** — brancher le déclenchement de la
capture sur le franchissement « healthz muet » de `host-probes.sh`, alert-only,
une fois par franchissement, en réutilisant la discipline `alert`/`clear_alert`
existante (`host-probes.sh:50-62`) — est **hors de ce périmètre** : les branches
et PR vont sur le dépôt où le code vit, et un pilote dispatché sur `mika` n'a pas
`gh` authentifié pour l'ouvrir. Son ouverture est un geste d'orchestrateur, et
son cadrage est écrit ici pour qu'il soit déposable sans rien rederiver : Yama
est absent de cet hôte (`/proc/sys/kernel/yama/ptrace_scope` n'existe pas), donc
gdb attache un process de même uid sans root — ce qui bloquait en juillet, où
seul root-Claude pouvait capturer. Question ouverte à y porter : en n=5, trois
PID orphelins wedgés coexistaient avec un child sain, donc
`/run/mika-spirit.pid` ne désigne pas nécessairement le process à capturer.

## 9. Sonde post-déploiement, et ses trois haltes

> **Préalable.** Ces mesures décrivent le **binaire servi**. Après `make deploy`,
> établir que le `mika-spirit` qui tourne est bien celui qu'on vient de bâtir
> avant toute conclusion (classe mika#2340).

Ce sont des **gestes opérateur**, sur l'hôte, hors du bac à sable : ils
déploient, redémarrent des services et capturent un process vivant — les trois
choses que le §3 sort du périmètre du pilote.

**S1 — la chaîne complète, sur une capture réelle.** Après `make deploy`,
capturer le spirit **vivant** (un process sain suffit : ce qu'on teste est la
symbolisation, pas le wedge), puis symboliser une frame.

```bash
scripts/mika-spirit-wedge-capture.sh $(pidof mika-spirit)
# puis, depuis le bundle : base PIE = 1re adresse du maps pour le binaire
addr2line -f -C -e ~/.local/bin/mika-spirit <runtime_addr - pie_base>
```

*Halte 1 — `addr2line` rend `??:0` sur le binaire déployé alors que l'AC1 était
verte dans le worktree.* Ne pas retoucher le profil : le binaire servi n'est pas
celui qui a été bâti. Établir le déploiement (`make install` a-t-il tourné ?)
**avant** toute conclusion sur le réglage.

**S2 — la garde mord, et elle regarde quelque chose.**
`make check-release-debuginfo` rend 0 sur `main` après merge ; puis, en posant
localement `strip = true`, rend non-zéro en nommant la ligne et son remède.

*Halte 2 — elle rend 0 dans les deux cas.* Elle est inerte : vérifier que le job
CI est bien câblé et que le bloc est trouvé (le refus d'anti-vacuité, exit 3, est
ce qu'il faut croire plutôt que réparer). *Une garde que personne n'a exercée se
lit exactement comme une garde qui marche* (mika#2205).

**S3 — le surcoût en service.** Taille de l'image `Dockerfile.agent` avant/après,
relevée une fois.

*Halte 3 — l'image franchit un seuil de déploiement cloud.* Le levier est
`split-debuginfo = "packed"` (§4), **pas** le retour de `strip = true` — qui
rouvrirait ce ticket en entier. Et le `.dwp` doit alors suivre le binaire : une
image mince dont les symboles sont restés sur la machine de build ne symbolise
rien.

## 10. Risques

- **Les images cloud grossissent.** `[profile.release]` est global, donc
  `Dockerfile.agent:46` et `Dockerfile.gateway:33` héritent du surcoût. Assumé :
  c'est le prix pour qu'un wedge chez un tenant soit diagnosticable. L'AC3 le
  chiffre, le repli existe si le chiffre est mauvais.
- **La capture est plus lourde et plus lente.** gdb charge davantage de tables.
  Sur un process déjà wedgé, sans conséquence — il ne va nulle part.
- **Faux sentiment de clôture.** Symboliser ne prévient **aucun** wedge ; cela
  raccourcit l'enquête. La prévention est portée par #1723 et #1724, et elle est
  déjà en place. Fermer ce ticket ne doit pas se lire comme « la classe est
  morte ».
- **Le repli déplace le problème sans le dire.** `packed` rend le binaire
  autosuffisant **faux** : sans son `.dwp`, il est aussi muet qu'aujourd'hui. Si
  le repli est pris, la doc de l'AC6 doit nommer où vit le `.dwp`, sinon on aura
  livré une procédure qui échoue au moment de servir.

## 11. Hors périmètre

- **Tout watchdog in-process.** Le corps du ticket (item 4) le demandait, mais un
  watchdog sur le runtime tokio ne peut structurellement pas voir cette classe :
  `spawn_engine_wedge_watchdog` (`liveness.rs:160`) est un `tokio::spawn`, donc
  il attend un worker que le wedge a précisément consommés ; et ses deux canaux
  de signal étaient eux-mêmes parkés à n=1 — le thread `tracing-appender` au
  frame partagé, les dix threads `mika-db` de même. Un observateur qui meurt avec
  ce qu'il observe n'est pas un observateur. La détection appartient au
  hors-process, où cm#126 l'a mise.
- **Tout redémarrage automatique.** RT#003, postérieur au corps de deux mois et
  ratifié par Vincent, le refuse : `liveness.rs:155-157`, « auto-recovery breaks
  the bounded-authority principle for process-restart operations. Operator
  decides remediation. »
- **Le fil sonde → capture**, qui vit dans `control-monitor` (§8, ticket
  compagnon).
- **Toute modification de code Rust** (DoD6).
- **La durée de build**, retirée avec son motif de fond au §3 — elle ne décide de
  rien et sa forme mesurée tue la session qui la prend.
- **Le déploiement et la capture d'un process vivant**, qui sont la sonde
  opérateur du §9.

## 12. Question de statut à porter à l'opérateur

mika#1719 est dans `bloqueurs-lancement` depuis le 2026-09-09T16:01Z. La mesure
du §1, refaite le 2026-09-30, montre que la panne qu'il décrit est **prévenue**
(#1723, #1724) et **détectée** (cm#126). Ce qui reste — la symbolisation — est un
raccourcisseur d'enquête, pas un empêcheur de panne.

Ce plan ne retire pas le ticket du jalon : c'est une décision opérateur. Il
fournit la mesure sur laquelle re-décider si ce reliquat gate encore le
lancement, ou s'il redescend d'un cran une fois livré. La question était déjà
posée dans le plan du 10/09 et n'a pas reçu de réponse ; elle est reportée telle
quelle plutôt que résolue par défaut.
