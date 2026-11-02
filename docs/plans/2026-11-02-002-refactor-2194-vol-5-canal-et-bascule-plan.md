# mika#2194 — vol 5 : le canal, et la bascule des deux parseurs

> **Ticket** : senara-solutions/mika#2194 (phase 3, ouverte dans ce ticket même
> par le réveil du 2026-11-02 ; gel total jusqu'au 2026-12-01).
> **Branche** : `refactor/2194/dispatch-lib-sortir-la-logique-du-bash-d`
> **Vol antérieur** : `docs/plans/2026-11-02-001-refactor-2194-vol-4-lecteur-unique-des-cinq-tiers-plan.md`
> (mergé, #3014) — son § 6 fixe le contrat du canal, qui n'est donc **pas**
> re-dérivé ici.
> **Merge** : `dispatch-lib.sh` est du substrat de boucle — le merge passe par le
> mot de Vincent, PR par PR.

---

## 0. Le vol, en une phrase

`_parse_disposition` et `_parse_verdict` cessent de décider : elles écrivent le
texte dans un fichier, appellent `mika arch-disposition`, et rendent ce que le
moteur a lu. Les 293 lignes de parseurs bash (`dispatch-lib.sh` l. 9796–10088)
partent. Les trois appelants de production ne changent pas d'une ligne.

Ce que ce vol **ne** fait pas, et qui est chiffré au § 6 : le scan anti-copie
étendu aux motifs de tier 1 (AC4) et le passage de la doctrine à « phase 3
livrée » (AC6). Ils forment le vol 6, qui est mergeable seul.

---

## 1. L'état mesuré, qui n'est plus celui du corps du ticket

Le corps décrit la tranche phase 3 **entière** (points 1–5, AC1–AC6). Deux
commits sont depuis sur la branche, et le plan est écrit contre le dépôt mesuré,
pas contre le corps.

**`593a745c5` (vol 4, #3014)** a livré :

- `crates/mika-agent/src/arch_disposition.rs` (27 770 o) — le lecteur unique des
  cinq tiers, dans l'ordre du bash, **sans aucun appelant de production** ;
- le corpus doré étendu aux cinq tiers : 24 corps sous
  `crates/mika-agent/tests/fixtures/arch_paraphrase_texts/` et 29 lignes
  d'attendus dans `expectations.tsv` (six colonnes TAB, les vides portant `-`) ;
- les deux lecteurs du corpus — `tests/arch_paraphrase_parity.rs` côté Rust,
  le bloc l. 14 944–15 087 de `test-dispatch-lib.sh` côté bash — et donc la
  **parité pre-switch**, mergée ;
- `DISPOSITION_WITHHELD_MARKER` passé en `pub(crate)`
  (`agent_loop/mod.rs` l. 9698). **Le R2 du vol 4 est fait : ce vol touche
  `agent_loop/mod.rs` pour zéro octet.** La marge du fichier sous le plafond de
  1 MiB reste donc de 1 641 octets, et aucun poste de ce plan ne la consomme.

**`45af2b338` (#3015)** a livré l'AC5 : le critère de famille des sous-commandes
de prédicat est remplacé dans `dispatch-lib-migration.md` (+41/−10, ce seul
fichier). L'AC5 est **close** — le corps le dit déjà (« non par le vol 5 »), et
le bearing de Prime du 2026-11-02 (b) le confirme : l'inscription s'est faite par
un geste séparé. **Ce plan ne rouvre pas l'AC5.**

### Deux divergences entre le corps et l'arbre, que le plan constate

**(i) La clause des « dix appels directs » de l'AC2 est déjà satisfaite.**
L'AC2 écrit : « Les dix appels directs de `_parse_disposition_fuzzy` et
`_parse_verdict_fuzzy` dans les tests (mesurés le 2026-11-02) ne disparaissent
pas avec ces fonctions : chacun devient un cas du corpus doré ». Mesure du jour,
sur l'arbre : **zéro appel direct subsiste** hors de `dispatch-lib.sh` — il ne
reste que des commentaires et les deux lignes de TSV. Le vol 4 a re-pointé le
lecteur du corpus vers les enveloppes en même temps qu'il écrivait la sixième
colonne. La clause est donc close par construction, et ce vol n'a rien à y
faire : il le dit et passe.

**(ii) `dispatch-lib.sh` n'est pas la seule suite concernée.** Voir § 4 : c'est
le point le plus mordant de ce vol, et le corps ne le mentionne pas.

### Ce que la parité du vol 4 devient ici, et ce qu'elle cesse d'être

Le § 1 du plan du vol 4 pose la loi, et son corollaire nomme ce vol :

> la même exécution du même test y est une **non-régression**, jamais une
> preuve. Son en-tête devra le dire.

Raison : avant la bascule, le bloc bash fait lire le corpus par un parseur bash
indépendant du Rust, et leur accord est une mesure. Après la bascule, le même
bloc fait lire le corpus par une enveloppe qui appelle le même moteur que le
lecteur Rust — **un lecteur comparé à lui-même est toujours d'accord**. Le bloc
reste utile (il garde la forme du canal, les codes de sortie, le fragment,
l'écriture du fichier fuzzy), mais il ne prouve plus la parité. La preuve est le
commit `593a745c5`, immuable sur la branche.

Conséquence opératoire, et c'est le geste le plus facile à oublier : la
**sentinelle pre-switch** du bloc, qui est aujourd'hui

```sh
M3_SWITCHED=$({ declare -f _parse_disposition; declare -f _parse_verdict; } 2>/dev/null | grep -c 'arch-disposition' || true)
# assertion : M3_SWITCHED == 0
```

a été écrite au vol 4 **pour rougir ici**. Elle n'est pas supprimée : elle est
**inversée** (`== 2`), et son commentaire change de camp — elle garde désormais
que la bascule a bien eu lieu dans les deux fonctions, et non qu'elle n'a pas
encore eu lieu. Une sentinelle supprimée perdrait l'information ; une sentinelle
inversée la retourne.

---

## 2. Le contrat du canal : repris du vol 4 § 6, pas re-dérivé

Le § 6 du plan du vol 4 a fixé le contrat **pour que ce vol n'ait pas à le
re-trancher**. Il est reproduit ici tel qu'il engage, et aucune de ses clauses
n'est rediscutée.

```
mika arch-disposition --pass <first|second> --response-file <chemin>
```

| code | signification | stdout | stderr |
|---|---|---|---|
| `0` | jeton lu | ligne 1 = le jeton, ligne 2 = le tier (`0`, `0b`, `1a`, `1b`, `2`) | le fragment décisif, pour le tier 2 seul |
| `1` | aucun jeton (UNPARSED) | ligne 1 **vide**, ligne 2 = `0` ou `-` | — |
| `≥2` | « je n'ai pas pu regarder » | — | `REFUSED (arch-disposition, mika#2194) <motif>: <détail>` |

Quatre clauses du § 6 qui pèsent sur l'implémentation :

1. **Toujours deux lignes** sur les codes `0` et `1`. Un lecteur bash qui fait
   `sed -n '2p'` sur une sortie d'une seule ligne lit le vide et ne peut pas le
   distinguer d'un tier absent. La règle `-`-jamais-vide du corpus (sixième
   colonne) s'applique ici à la **ligne 2 seule** : la ligne 1 est
   légitimement vide au code `1`, c'est sa signification.
2. **Le corps passe par FICHIER, jamais par argv** — classe de portée de
   guillemets cpp#157, et doctrine § 2 du ticket.
3. **Le refus atteint le `RESULT` du callback.** Une stderr de pré-pilote est
   structurellement perdue sur un dispatch réussi (classe mika#2050) : un refus
   qui n'y voyage pas se lit comme un UNPARSED silencieux, ce que l'AC3 interdit
   nommément. Le § 3 ci-dessous corrige le *mécanisme* que le vol 4 avait
   supposé.
4. **Aucun repli bash.** Un repli serait une seconde implémentation — la chose
   exacte que ce vol retire.

**La forme du fragment est contrainte par un lecteur existant.** Le bloc de
parité bash extrait le fragment avec

```sh
sed -n "s/^.*(matched: '\(.*\)')\$/\1/p"
```

parce que le bash écrit aujourd'hui, sur stderr :

```
_parse_disposition_fuzzy: mapped paraphrased disposition → ESCALATE (matched: 'human review')
```

L'AC2 exige que les attentes des tests existants ne changent pas. La stderr du
canal doit donc se terminer par `(matched: '<fragment>')`, le fragment étant **le
texte apparié tel qu'il figure dans l'entrée** — casse comprise, et non le motif
de la table. `arch_disposition::Decision::fragment` est déjà une tranche de
l'entrée, donc cette propriété est portée par le vol 4 et il n'y a rien à
recalculer : il faut seulement ne pas la perdre à l'écriture.

---

## 3. Le refus traverse un `$(...)` — correction d'une analogie du vol 4

Le § 6 du vol 4 écrit que « le refus voyage jusqu'au `RESULT` du callback, comme
`_PLAN_CALLOUT_REFUSAL` le fait déjà ». **L'analogie est juste sur la
destination, fausse sur le mécanisme**, et la différence est structurelle.

Mesuré : `_PLAN_CALLOUT_REFUSAL` est posée aux lignes 4856
(`_committed_plan_on_branch`) et 13063 (`_detect_plan_on_branch`) — **dans le
shell principal**. `_extract_plan_path` est bien appelée en `$(...)`, mais c'est
son *code de retour* qui franchit la frontière, et c'est l'appelant, hors
sous-shell, qui pose la variable.

Les trois appelants de `_parse_disposition` / `_parse_verdict` sont dans l'autre
configuration :

```sh
disposition=$(printf '%s' "$content1" | _parse_disposition)     # l. 11286
verdict=$(printf '%s' "$content2" | _parse_verdict)             # l. 11379
verdict_iter=$(printf '%s' "$content2_iter" | _parse_verdict)   # l. 11523
```

L'enveloppe **est** le sous-shell. Une globale qu'elle poserait mourrait avec
lui — et le fichier `$_DISPOSITION_FUZZY_FILE` existe précisément pour cette
raison, que le commentaire de `dispatch-lib.sh` l. 9779-9786 énonce en propres
termes : « bash subshells (`$(...)`) cannot set parent variables, so we use a
tmpfile to communicate the flag across the subshell boundary ».

**Décision : le refus emprunte le même canal que le booléen fuzzy.** Un second
fichier, `$_ARCH_DISPOSITION_REFUSAL_FILE`, déclaré au même endroit et sous la
même forme (`"${TMPDIR:-/tmp}/.dispatch-lib-arch-refusal-$$"`), écrit par
l'enveloppe sur un code `≥2`, lu par `_annex_arch_disposition_refusal` depuis le
shell principal au moment du callback.

C'est cohérent avec la précondition tranchée dans le corps du ticket — « le
fichier reste, et c'est le shell qui l'écrit à partir de la sortie du moteur »,
au titre de C3 : ce que seul le shell connaît reste au shell. Un canal de
franchissement de sous-shell est de la glue, pas une décision. En ajouter un
second du même genre n'étend pas la glue conceptuellement ; la retirer est le
travail de la phase 4, qui retirera les deux ensemble.

**Point de vigilance nommé** : les deux fichiers portent `$$` et sont nettoyés
par le trap `EXIT` que `dispatch-lib` installe déjà. Le second doit entrer dans
le même trap, sinon il laisse un résidu par dispatch — et un résidu lu par un
`$$` recyclé annoncerait un refus qui n'a pas eu lieu. Le nettoyage n'est donc
pas de l'hygiène, c'est une condition de justesse.

---

## 4. La casse de câblage CI, intrinsèque à la bascule

C'est la découverte la plus mordante de ce grooming, et le corps du ticket ne la
porte pas.

**Le fait.** `skills/bundled/_shared/tests/test_parse_disposition.sh` (361
lignes, **55 appels** à `_parse_disposition` / `_parse_verdict`) source
`dispatch-lib.sh` directement et n'a **ni** résolution de `MIKA_BIN` **ni** porte
de skip. Elle est atteinte par un seul site de CI : `.github/workflows/ci.yml`
l. 703, dans le job `dispatch-lib-offline-suites` — un job dont les étapes sont
`actions/checkout` suivi d'invocations `bash …` nues, **sans aucune étape
cargo**. Le commentaire de l'étape dit sa raison d'être : « a pure function over
strings. Nothing in this repository is cheaper to run in CI. »

Après la bascule, chacun de ces 55 appels a besoin du binaire `mika`. Dans ce
job il n'existe pas. Les ~80 assertions de la suite deviennent rouges — et
l'AC2 exige l'inverse en propres termes : « passent **sans modification de leurs
attentes** ».

Le même problème, en plus petit, touche `test-dispatch-lib.sh` : ~30 appels
d'enveloppe en ligne aux l. 718–845, et les deux appels du bloc de parité
(l. 15 044 / 15 047), ne passent par aucun `mika_bin_or_skip`. Ce fichier est
mieux placé — il tourne via `make test-dispatch-lib` (ci.yml l. 85) **après**
`cargo test` (l. 78), ce qui est exactement la raison pour laquelle sa
résolution de `MIKA_BIN` trouve `target/debug/mika`.

**Volume mesuré** : 55 + 32 ≈ **87 démarrages de `mika` ajoutés** à travers les
suites, pour 29 lignes de corpus sur 24 corps. C'est la sonde S3 du corps vue
depuis les tests, et elle est hors tranche en tant que mesure — mais le plan la
chiffre, parce qu'un ordre de grandeur inattendu ici serait le signe que le
placement dans `main.rs` a été perdu.

**Remède retenu, en deux gestes.**

1. **L'étape l. 703 déménage dans le job `Check`**, après `cargo test`. C'est le
   job où `target/debug/mika` existe déjà, pour la raison même qui fait marcher
   `test-dispatch-lib.sh`. Le déménagement préserve l'atteignabilité au sens de
   `suite-wiring-lint` : la suite reste nommée par un workflow, donc elle ne
   devient pas une entrée `unwired` du registre (et le registre est à zéro depuis
   mika#2793 — y ouvrir une ligne coûterait son propre ticket de
   Fire-Disposition).
2. **La résolution de `MIKA_BIN` et la porte `mika_bin_or_skip` deviennent
   partagées.** Le bloc vit aujourd'hui aux l. 55–95 de `test-dispatch-lib.sh`.
   Il part dans `skills/bundled/_shared/tests/lib-mika-bin.sh`, sourcé par les
   deux suites. **Pas une copie** : ce ticket existe pour retirer une seconde
   implémentation, en introduire une par le remède serait se contredire. La porte
   couvre le cas **développeur** (un `bash` à la main sans build préalable) et
   garde la propriété mika#2149 — un SKIP est **bruyant**, avec sa troisième
   colonne, parce qu'une porte qui n'a pas pu s'armer ne doit jamais se lire
   verte.

**Deux remèdes écartés, avec leur raison.**

- *Ajouter une étape cargo au job offline.* Détruit la prémisse et le coût du
  job, dont la valeur entière est d'être le moins cher du dépôt.
- *Skip seul, sans déménagement.* La suite ne tournerait **jamais** en CI : un
  détecteur qui ne mesure rien, classe mika#2205, et le genre de vert qui passe
  la revue par construction.

---

## 5. Les livrables

### R1 — `crates/mika-cli/src/commands/arch_disposition.rs` (nouveau)

Modelé sur `plan_callout.rs`, qui est le gabarit du canal à trois codes dans ce
dépôt :

- `pub const EXIT_UNREADABLE: i32 = 2;`
- `const REFUSAL_PREFIX: &str = "REFUSED (arch-disposition, mika#2194)";`
- un énuméré d'issue à trois bras — jeton lu / aucun jeton / illisible avec
  `reason: &'static str` et `detail: String` ;
- `pub fn run(response_file: &Path, pass: ArchPass) -> i32` : lit le fichier,
  délègue à `mika_agent::arch_disposition::decide`, écrit **toujours deux
  lignes** sur stdout, le fragment sur stderr au tier 2, et rend le code.

La **décision** n'est pas réimplémentée : elle est dans `arch_disposition.rs`
côté `mika-agent` depuis le vol 4, et `lib.rs` l'expose déjà (`pub mod
arch_disposition;`). Ce module est un canal, et il ne contient aucune règle de
tier. C'est ce qui rend le poste petit, et c'est voulu : la seule chose qu'un
canal peut faire de mal, c'est perdre une information en la formatant.

Tests inline : les trois codes ; le tier sur la ligne 2 pour chacun des cinq
tiers ; la forme `(matched: '…')` du fragment ; et l'**AC3 aux deux polarités
dans le même corps** — un fichier illisible rend un refus nommé, une réponse
saine n'en rend aucun. Les deux polarités ensemble, parce qu'un test qui ne
vérifie que le refus ne distingue pas « refuse quand il faut » de « refuse
toujours » (gabarit `mika2194_un_chemin_multiligne_est_refuse_avec_son_motif` /
`mika2194_un_corps_sain_nest_jamais_refuse`).

### R2 — `crates/mika-cli/src/cli.rs`

- `#[command(name = "arch-disposition")] ArchDisposition(ArchDispositionArgs)` ;
- `pub struct ArchDispositionArgs { pub pass: ArchPass, pub response_file: PathBuf }`
  avec `--pass` et `--response-file` en options longues, **sans variante
  positionnelle par construction** — même raison que `PlanCalloutArgs` : un
  positionnel réintroduirait la classe de portée de guillemets ;
- la variante entre dans les **trois `match` exhaustifs** (l. 140, 172, 205),
  qui existent pour que l'ajout d'une variante soit une erreur de compilation
  tant qu'elle n'a pas été classée. C'est le filet qui rend ce poste sûr : on ne
  peut pas oublier un bras.

`--pass` se mappe sur `ArchPass`, que `arch_disposition.rs` ré-exporte déjà
(`pub use crate::arch_paraphrase::ArchPass;`) — pas de second énuméré.

### R3 — `crates/mika-cli/src/main.rs`

Un bras de **chemin court** à côté de `Token`, `CredentialHelper` et
`PlanCallout` (l. 34), donc **avant** `dotenv`, `Settings`, `home`, la DB et le
réseau ; et la variante dans le `unreachable!()` de la l. 470. Le placement
n'est pas cosmétique : c'est la réponse du corps du ticket à la sonde S3 — « si
S3 dépasse environ 500 ms, le remède est le placement dans `main.rs`, jamais un
cache ». Le remède est donc pris d'avance, et un cache reste exclu.

### R4 — `skills/bundled/_shared/dispatch-lib.sh`

**Retiré** : l. 9796–10088, soit **293 lignes** — `_parse_disposition_fuzzy`,
`_parse_verdict_fuzzy`, `_engine_escalation_line`, `_parse_disposition` et
`_parse_verdict`, tables de paraphrase comprises.

**Écrit** : deux enveloppes, une par fonction. Chacune :

```
texte ← stdin → mktemp
mika arch-disposition --pass <first|second> --response-file "$tmp"  (stdout capturé, rc capturé)
rm -f "$tmp"
rc 0 → ligne 1 sur stdout ; printf '1'|'0' > $_DISPOSITION_FUZZY_FILE selon ligne 2 == 2
rc 1 → rien sur stdout ; printf '0' > $_DISPOSITION_FUZZY_FILE ; return 1
rc ≥2 → motif > $_ARCH_DISPOSITION_REFUSAL_FILE ; printf '0' ; stderr nommée ; return rc
```

Trois détails de conservation, chacun mesuré, chacun invisible si on ne le
cherche pas :

- **Le `printf '0'` de tête reste.** Les deux fonctions actuelles écrivent `0`
  dans le fichier fuzzy **en entrée**, pas seulement en sortie. Sans cela, un
  appel qui échoue tôt laisserait le `1` de l'appel précédent, et la ligne de
  trail porterait un `(fuzzy)` emprunté à un autre tour.
- **La divergence `|| true` est préservée.** Le tier 1a de `_parse_disposition`
  porte un `|| true` (l. ~9961) que celui de `_parse_verdict` n'a pas (l. 10047).
  L'enveloppe ne doit pas harmoniser : les codes de retour des deux fonctions
  sont ce que les trois appelants lisent, et l'AC2 porte sur eux.
- **`_parse_verdict` n'a délibérément pas de bras `ITERATE` en tier 1b.** Cette
  asymétrie est portée par `arch_disposition::carry_over` côté Rust
  (`(Second,"READY")→GROOMED`, `(Second,"ESCALATE")→ESCALATE`, rien d'autre), donc
  elle survit à la bascule sans que l'enveloppe ait à en savoir quoi que ce soit.
  C'est noté ici pour qu'on ne la « corrige » pas en relisant les deux
  enveloppes côte à côte et en les trouvant trop semblables.

**Ajouté** : `_ARCH_DISPOSITION_REFUSAL_FILE` déclaré à côté de
`_DISPOSITION_FUZZY_FILE` (et entrant dans le même trap `EXIT`), plus
`_annex_arch_disposition_refusal()` sur le modèle exact de
`_annex_plan_callout_refusal` (l. 12839) : idempotent par `grep -qF`, appendant
`REFUSED (arch-disposition, mika#2194) <motif>` au `RESULT`, invoqué depuis
`_deliver_callback` juste après son jumeau.

**Inchangé** : les trois appelants de production, l. 11286/11288, 11379/11381,
11523/11525. C'est la forme strangler, et c'est la mesure du succès de ce vol.

### R5 — `skills/bundled/_shared/test-dispatch-lib.sh`

Quatre gestes :

1. **La sentinelle pre-switch est inversée** (§ 1) : `M3_SWITCHED == 2`, et son
   commentaire dit qu'elle garde la bascule au lieu de son absence.
2. **L'en-tête du bloc de parité est réécrit** : il dit, en propres termes, que
   ce bloc est désormais une **non-régression de forme du canal** et non une
   preuve de parité, que la preuve est le commit `593a745c5`, et pourquoi (un
   lecteur comparé à lui-même est toujours d'accord). Les trois assertions
   d'anti-vacuité et l'égalité `M3_CASES == M3_DECLARED` restent : elles portent
   sur le fait que le corpus est bien lu, ce qui reste vrai et reste nécessaire.
   Le `|| true` reste absent de l'appel, délibérément, pour qu'un `≥2` ne soit
   pas avalé.
3. **La garde de dérive de marqueur est inversée** (l. 6430–6462). Elle assère
   aujourd'hui que `dispatch-lib.sh` **porte** le littéral du marqueur
   (mika#2037) et celui de l'ancre moteur (mika#2338) — ce que ce vol retire.
   Elle devient : le littéral n'est **plus** dans le bash, **et** la constante
   Rust a un lecteur unique. Les deux moitiés ensemble, car la supprimer
   verdirait un arbre où le tier 0 aurait été recopié ailleurs, et la première
   moitié seule verdirait un arbre où il aurait disparu des deux côtés.
   L'extraction par `grep -oE 'const DISPOSITION_WITHHELD_MARKER: &str = "[^"]+"'`
   est conservée telle quelle : non ancrée, elle survit au `pub(crate)` que le
   vol 4 a posé.
4. **Les blocs d'appels nus passent la porte** : les ~30 appels en ligne des
   l. 718–845 et les deux du bloc de parité derrière `mika_bin_or_skip`, pour le
   cas développeur (§ 4).

Plus l'**assertion d'absence des tables de tier 2** — voir § 6 pour ce qui en
est renvoyé, et `## Fire-Disposition` pour sa disposition.

### R6 — `skills/bundled/_shared/tests/lib-mika-bin.sh` (nouveau) + `test_parse_disposition.sh`

Le bloc de résolution de `MIKA_BIN` (PATH, puis `target/debug/mika`, puis
`target/release/mika`) et `mika_bin_or_skip()` sortent de
`test-dispatch-lib.sh` l. 55–95 vers ce fichier, que les deux suites sourcent.
`test_parse_disposition.sh` conserve son `trap 'rm -f "$_DISPOSITION_FUZZY_FILE"' EXIT`
et ses 55 appels **avec leurs attentes inchangées** (AC2), et gagne le trap du
second fichier (§ 3).

### R7 — `.github/workflows/ci.yml`

L'étape `bash skills/bundled/_shared/tests/test_parse_disposition.sh` quitte
`dispatch-lib-offline-suites` pour `Check`, après `cargo test` (§ 4). Le
commentaire de l'étape est réécrit : la suite n'est plus « la moins chère du
dépôt », elle a désormais une dépendance de build, et le commentaire doit le
dire, sinon le prochain qui voudra économiser la ramènera.

### R8 — `crates/mika-agent/tests/arch_paraphrase_parity.rs`

En-tête seul. Il annonce aujourd'hui que la colonne `tier` a un lecteur unique
pre-switch et que la ligne 2 de stdout est ce que le vol 5 ouvre. Il devient le
constat que la ligne 2 est ouverte, et que ce fichier reste le lecteur Rust du
corpus — le seul des deux dont l'accord avec le corpus mesure encore quelque
chose, puisqu'il est écrit contre `decide` et non contre le canal. Aucune
assertion ne change.

### R9 — `scripts/canonical-tokens.tsv`

Retrait des lignes de **classe A** devenues fausses : l. 85–86
(`Disposition:` / `Verdict:` sur `::_parse_disposition_fuzzy` /
`::_parse_verdict_fuzzy`) et les huit lignes de l. 185–192
(`::_parse_disposition` / `::_parse_verdict`). La prose du bloc du vol 4, qui
annonce que « l'inventaire redevient ce qu'il doit être — UN site par jeton »,
devient un constat.

Deux bornes sur ce poste :

- **Les quatre lignes de `arch_paraphrase.rs::paraphrase` (l. ~211–217)
  restent.** Les tables de paraphrase existent toujours — elles ont migré, elles
  n'ont pas disparu. « UN site par jeton » veut dire un site **bash** de moins,
  pas un site Rust de moins. Les supprimer rendrait l'inventaire faux dans
  l'autre sens.
- **Le risque est faible et c'est mesuré** :
  `canonical_tokens.rs:283 fn mika2201_every_match_site_is_declared` construit
  sa table de déclarés **à partir des lignes de classe B seules**. Les éditions
  de classe A sont donc documentaires, pas sous lint. L'assertion
  `!declared.is_empty()` du même test reste satisfaite (la classe B n'est pas
  touchée), et la ligne 406 (`ENGINE_ESCALATION_ANCHOR`, classe B) non plus.

### R10 — hors compte : rien dans `docs/architecture/`

L'AC6 part au vol 6 (§ 6). Ce vol n'écrit pas un mot de doctrine.

---

## 6. Ce que ce vol renvoie, chiffré, et pourquoi

Le plan du vol 4 avait fixé le plancher du vol 5 à « le canal + les deux
enveloppes + le scan anti-copie », en raisonnant que « livrer les enveloppes
sans le scan laisserait les tables mortes dans le fichier sans rien pour le
dire ». **La mesure de la taille (ci-dessous) ne tient pas ce plancher**, et le
plan le dit plutôt que de le contourner.

Deux choses ont changé depuis cette phrase, et la seconde est la raison de fond :

1. **La taille mesurée.** Le vol 4 chiffrait le vol 5 à ≈ 278 lignes. Il n'avait
   vu ni la casse de câblage CI (§ 4, ≈ 75 lignes de remède) ni le second canal
   de refus (§ 3, ≈ 25 lignes) — deux découvertes de ce grooming. Avec elles et
   le scan complet, le vol passe à ≈ 466 lignes, au-dessus de la borne de 429.
2. **L'inquiétude du vol 4 ne décrit pas ce vol.** « Les tables mortes dans le
   fichier » serait l'état d'un vol qui ajouterait les enveloppes *à côté* des
   tables. Ce vol **retire** les 293 lignes (R4). Il n'y a pas de table morte :
   il y a une absence. Le rôle du scan anti-copie n'est donc pas de constater
   l'état livré — c'est d'empêcher un **retour**.

**Ce que ce vol garde malgré tout du scan.** Pas rien, et pas le tout : une
assertion d'absence sur les **littéraux discriminants de la table de tier 2**
(`human review`, `cannot proceed`, `out of scope for`, `needs revision`,
`another pass`, `address the following`, `concerns that require`, `ship it`,
`good to go`, `plan is clean`), sur les lignes de **commande** de
`dispatch-lib.sh`, commentaires retirés d'abord — avec son contrôle de bonne foi
positif.

Cinq des quinze chaînes de la table sont **écartées de la population par nom**,
avec leur raison : `escalate`, `fundamental`, `revise`, `proceed` et `dispatch`
sont des mots courants de ce fichier (`dispatch` y est partout), et une
assertion qui les inclurait serait **rouge à la naissance, donc désarmée** — le
piège que le gabarit de la phase 2 documente aux l. 11 130–11 150. Ce n'est pas
une allowlist d'exemption : c'est la définition de la population, la même
distinction que fait déjà le TSV de mika#2201 en ne déclarant que des *sites de
match*.

**Renvoyé au vol 6** (≈ 60 lignes comptées, mergeable seul) :

| livrable du vol 6 | lignes estimées |
|---|---|
| AC4 — scan anti-copie étendu aux motifs de **tier 1** (`Disposition:` / `Verdict:`), avec le discriminant « outil de lecture », l'assertion zéro et ses trois contrôles de bonne foi | 60 |
| AC6 — `dispatch-lib-migration.md` §§ 6–7 : phase 3 à « livrée », ligne du vol 4 dans la table du § 6, décision sur le canal | hors compte (`docs/`) |

Les motifs de **tier 1** ont besoin du discriminant complet, et c'est ce qui
rend ce poste plus lourd que l'assertion ci-dessus : `Disposition:` et `Verdict:`
sont **écrits** par le trail de vérdict et cités dans les commentaires de
doctrine de ce même fichier. Le prédicat doit donc porter sur la présence d'un
outil de lecture (`grep|sed|awk|=~|case|expr`) sur la même ligne, exactement
comme `M2194_READER` le fait pour le callout — avec la population non vide et les
contrôles positif et négatif qui distinguent « zéro lecteur » de « le prédicat
ne regarde rien » (classe mika#2205). `m2194_read_sites()` est paramétrée par son
aiguille à cette occasion, plutôt que recopiée.

**AC6 va avec AC4, et pas ailleurs.** La doctrine ne déclare pas livré ce qui ne
l'est pas : « phase 3 livrée » suppose l'AC4 fermé. Les mettre dans le même vol
est la seule séquence qui ne fasse pas mentir le § 6 de la doctrine pendant une
PR.

**Ordre de commit, et le plancher de chaque vol.**

- **Vol 5 d'abord**, obligatoirement : le scan du vol 6 porte sur l'absence que
  le vol 5 crée. Lancé avant, il serait rouge à la naissance.
- **Plancher du vol 5** : le canal (R1) + le câblage (R2, R3) + les deux
  enveloppes (R4) + le remède CI (R6, R7). Retirer le remède CI livre une CI
  rouge sur une suite que l'AC2 déclare intouchée — ce n'est pas un vol réduit,
  c'est un vol faux.
- **Plancher du vol 6** : le scan de tier 1 + la doctrine. Aucun des deux seul.

Total des deux vols : ≈ 466 lignes, soit ≈ 1,1× la borne par vol — c'est la
mesure qui impose le découpage.

Tracked in: senara-solutions/mika#2194

Le ticket parent est le porteur légitime du reste : la décision opérateur du
2026-09-30T19:03:44Z dit en propres termes que *« ce ticket parent reste ouvert
et les porte »*, et le réveil du 2026-11-02 a ouvert la phase 3 **dans ce ticket
même, sans nouveau ticket** (gel total jusqu'au 2026-12-01). Le bearing de Prime
du 2026-11-02 (b) le redit pour la doctrine : « gel : aucun ticket neuf ».

---

## 7. Hors tranche, redit ici pour qu'il n'y entre pas

- **Phase 4** : retirer `$_DISPOSITION_FUZZY_FILE` lui-même — et, désormais, son
  jumeau de refus du § 3. Les deux partent ensemble, ou aucun.
- **B1 — aucune harmonisation de tolérance.** Le comportement est porté à
  l'identique, tier 2 compris : il reste anglais et reste **inerte sur les
  réponses françaises** (classe `french-inert` du corpus). Le corriger serait un
  changement de comportement, qu'un autre vol ferait. Les corps `french-inert-*`
  du corpus sont là pour que ce vol ne le fasse pas par mégarde.
- **Les autres prédicats de `dispatch-lib.sh`** (`_plan_header_claimed_issues`,
  `_branch_reuse_gate`…) : ils ne satisfont pas C3 aujourd'hui.
- **AC5** : close par #3015 (§ 1). Ce vol ne la rouvre pas.
- **La sonde S3** sur le coût d'un démarrage de `mika`. Ce vol ajoute un
  démarrage par passe d'architecte et le dit (§ 4 en chiffre 87 pour les
  suites). Si S3 dépasse ≈ 500 ms, le remède est le placement dans `main.rs` —
  déjà pris en R3 — et jamais un cache.
- **`agent_loop/mod.rs`** : zéro octet (§ 1). Si un poste de ce plan se met à
  vouloir y écrire, c'est hors tranche et il faut le signaler avant de le faire :
  la marge est de 1 641 octets sous le plafond de 1 MiB, et
  `crates/mika-common/tests/claude_md_size.rs` n'est pas le seul plafond du
  dépôt.

---

## Fire-Disposition

Ce vol livre des détecteurs : l'assertion AC3 aux deux polarités (R1),
l'assertion d'absence des littéraux de tier 2 (R5/§ 6), la sentinelle pre-switch
inversée et la garde de dérive de marqueur inversée (R5). La section est donc
requise.

**Option retenue : (a) exception nommée en allowlist — et, pour trois des quatre
détecteurs, l'allowlist est vide.**

**Les trois détecteurs qui atterrissent à zéro violation, armés.**

1. **L'assertion AC3** porte sur du code que ce vol écrit : il n'existe aucune
   violation antérieure possible. Armée, à zéro, sans exception.
2. **La sentinelle pre-switch inversée** (`M3_SWITCHED == 2`) est verte dès que
   R4 est livré, et rouge sinon. Elle n'a pas d'allowlist parce qu'elle n'a pas
   de population : elle lit deux fonctions nommées.
3. **La garde de dérive inversée** est verte dès que les 293 lignes sont
   retirées. Même raison.

Si l'un des trois est rouge à la livraison, ce n'est pas une violation
préexistante à exempter — c'est que R4 n'est pas fini, et le remède est de le
finir.

**Le détecteur qui a une population, et son exception nommée.**

L'assertion d'absence des littéraux de tier 2 (§ 6) porte sur un fichier de
≈ 300 Ko. Sa population est définie par **écart nommé** : cinq des quinze
chaînes de la table — `escalate`, `fundamental`, `revise`, `proceed`,
`dispatch` — sont écartées, chacune avec sa raison écrite sur sa ligne (ce sont
des mots courants de `dispatch-lib.sh`, `dispatch` y étant omniprésent). Les dix
autres sont discriminantes et l'assertion est un **zéro**.

Trois propriétés de cet écart, qui sont ce qui le distingue de l'allowlist que
mika#2309 condamne :

- Il est **grep-visible** : les cinq chaînes sont écrites en clair dans le test,
  pas déduites d'un prédicat.
- Il nomme la **donnée précise** (la chaîne), pas un fichier ni une plage de
  lignes.
- Il porte son **assertion auto-nettoyante** : le test assère que la population
  des dix chaînes discriminantes est **non vide** dans l'arbre d'avant le retrait
  — mesuré depuis `git show 593a745c5:skills/bundled/_shared/dispatch-lib.sh`,
  donc depuis une révision immuable. Si un jour les dix chaînes ne sont plus dans
  cette révision, c'est que l'assertion a cessé de mesurer son objet, et elle
  rougit. C'est le terme d'anti-vacuité : sans lui, un zéro et un prédicat aveugle
  se lisent pareil.

**Le suivi du reste.** Le scan anti-copie étendu aux motifs de **tier 1** est
nommé, chiffré (≈ 60 lignes) et tracké au § 6 sur
`senara-solutions/mika#2194` — le ticket parent, qui porte les vols par décision
opérateur du 2026-09-30T19:03:44Z, sous le gel « aucun ticket neuf » du bearing
de Prime du 2026-11-02. Ce n'est pas un détecteur livré désarmé : c'est un
détecteur **non livré**, dont la raison de ne pas l'être est la borne par vol et
non une violation qu'il aurait trouvée.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-cli/src/commands/arch_disposition.rs` (le canal : trois codes, deux lignes toujours, fragment) | 70 |
| ses tests inline (cinq tiers sur la ligne 2 + forme du fragment + AC3 aux deux polarités) | 55 |
| `crates/mika-cli/src/cli.rs` (variante, `ArchDispositionArgs`, les 3 bras exhaustifs) | 20 |
| `crates/mika-cli/src/main.rs` (chemin court + `unreachable!()`) | 6 |
| `skills/bundled/_shared/dispatch-lib.sh` (2 enveloppes, canal de refus, `_annex_arch_disposition_refusal` ; retrait de 293 lignes) | 95 |
| `skills/bundled/_shared/test-dispatch-lib.sh` (sentinelle inversée, garde de dérive inversée, porte `mika_bin_or_skip`, en-tête de non-régression, assertion d'absence tier 2) | 85 |
| `skills/bundled/_shared/tests/lib-mika-bin.sh` (extrait, sourcé par les deux suites) + `test_parse_disposition.sh` | 40 |
| `.github/workflows/ci.yml` (la suite déménage dans `Check`) | 5 |
| `crates/mika-agent/tests/arch_paraphrase_parity.rs` (en-tête post-switch) | 12 |
| `scripts/canonical-tokens.tsv` (retrait des 10 lignes de classe A + la prose du vol 4 en constat) | 18 |

Total estimé : 406 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

### Pourquoi ce vol tient, et le poste qui cède

406 ≤ 429, avec 23 lignes de marge. Les deux postes qui pourraient déraper sont
bornés par leur nature :

- **Le canal (70)** ne contient **aucune règle de tier** : les cinq tiers sont
  dans `arch_disposition.rs` côté `mika-agent` depuis le vol 4, et `lib.rs`
  l'exporte déjà. Le module lit un fichier, appelle `decide`, formate deux
  lignes. Son gabarit mesuré, `plan_callout.rs`, fait 321 lignes **tests et
  doctrine compris** — et une bonne moitié y est l'essai sur la politique des
  clôtures, qui n'a pas d'équivalent ici.
- **Les enveloppes (95)** remplacent 293 lignes par deux corps de ≈ 25 lignes
  chacun. Le reste du poste est le canal de refus (§ 3) et l'annexe, dont le
  gabarit `_annex_plan_callout_refusal` fait 15 lignes, mesuré.

**Si le portage dépasse, le poste qui cède est « tests inline du canal ».** Les
cas qu'il perdrait — un tier par ligne 2 — sont déjà couverts par le corpus doré,
qui est la vraie mesure, et qui est lu de deux côtés. Les deux polarités de
l'AC3 ne cèdent **pas** : elles sont le critère d'acceptation, et elles n'ont
aucun autre lecteur.

---

## Definition of Done

1. `mika arch-disposition --pass <first|second> --response-file <chemin>` existe,
   rend les trois codes du § 2, écrit **toujours deux lignes** sur stdout aux
   codes `0` et `1`, et le fragment du tier 2 sur stderr sous la forme
   `(matched: '<texte apparié>')`.
2. Le corps de la réponse passe par **fichier**. Aucune variante positionnelle
   n'existe sur `ArchDispositionArgs`.
3. La sous-commande est branchée dans le **chemin court** de `main.rs`, avant
   `dotenv` / `Settings` / `home` / DB / réseau, et dans les trois `match`
   exhaustifs de `cli.rs`.
4. `_parse_disposition` et `_parse_verdict` sont des enveloppes. Les lignes
   9796–10088 de `dispatch-lib.sh` (293 lignes, les deux fonctions fuzzy et leurs
   tables comprises) sont retirées. **Aucun repli bash n'est écrit.**
5. Les trois appelants de production (l. 11286, 11379, 11523) et le contrat de
   `_disposition_was_fuzzy` sont **inchangés**. Le `printf '0'` d'entrée, le
   `|| true` asymétrique du tier 1a et l'absence de bras `ITERATE` en tier 1b de
   `_parse_verdict` sont préservés.
6. Un code `≥2` produit un refus nommé dans le `RESULT`, jamais un UNPARSED
   silencieux : `_ARCH_DISPOSITION_REFUSAL_FILE` traverse le `$(...)`,
   `_annex_arch_disposition_refusal` l'annexe depuis `_deliver_callback`, et le
   fichier entre dans le trap `EXIT` existant.
7. Un test épingle les **deux polarités** de l'AC3 dans le même corps : refus sur
   moteur absent ou fichier illisible, **rien** sur une réponse saine.
8. La sentinelle pre-switch est **inversée** (`M3_SWITCHED == 2`) et la garde de
   dérive de marqueur est **inversée** (le littéral n'est plus dans le bash **et**
   la constante Rust a un lecteur unique). Aucune des deux n'est supprimée.
9. L'en-tête du bloc de parité bash dit qu'il est désormais une non-régression de
   forme du canal et **non** une preuve de parité, et nomme `593a745c5` comme la
   preuve. Ses trois assertions d'anti-vacuité et `M3_CASES == M3_DECLARED`
   restent.
10. L'assertion d'absence des dix littéraux discriminants de tier 2 est verte,
    avec son contrôle de bonne foi positif, son terme d'anti-vacuité contre
    `593a745c5`, et les cinq chaînes écartées nommées avec leur raison.
11. **`test_parse_disposition.sh` passe en CI** : l'étape a déménagé dans `Check`
    après `cargo test`, ses 55 appels gardent leurs attentes **inchangées**, et la
    résolution de `MIKA_BIN` + `mika_bin_or_skip` est **partagée** via
    `_shared/tests/lib-mika-bin.sh` — pas copiée. Un SKIP reste bruyant
    (mika#2149).
12. Les ~30 appels d'enveloppe en ligne de `test-dispatch-lib.sh` (l. 718–845) et
    les deux du bloc de parité passent la porte `mika_bin_or_skip`.
13. `scripts/canonical-tokens.tsv` a perdu ses deux lignes 85–86 et les huit de
    185–192. Les quatre lignes de `arch_paraphrase.rs::paraphrase` sont
    **conservées**. `mika2201_every_match_site_is_declared` et la ligne 406 de
    classe B passent.
14. `agent_loop/mod.rs` est modifié de **zéro octet**.
15. `cargo test`, `cargo clippy`, `cargo fmt --check`, `make test-dispatch-lib`,
    `make verify-bundled-skills`, `scripts/check-suite-wiring.sh` (dont le champ
    `unwired=` reste à 0) et `scripts/verify-pipeline.sh origin/main` passent.
16. Le corps de PR dit explicitement que **le ticket mika#2194 n'est pas clos** —
    aucun `Closes #2194`. Il porte la ligne `Tracked in:
    senara-solutions/mika#2194` pour le vol 6 (AC4 + AC6), le chiffrage du § 6, et
    `Code review: delegated to mika-qa (mika#2694)`.
17. Aucun mot de `docs/architecture/dispatch-lib-migration.md` n'est modifié :
    l'AC6 est au vol 6, parce que la doctrine ne déclare pas livré ce qui ne
    l'est pas.

---

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#2194 (section « Critères
d'acceptation » de la tranche phase 3), verbatim. Leur répartition entre vol 5 et
vol 6 est au § 6 de ce plan ; elle ne modifie pas leur énoncé.

- **AC1 : parité dorée sur les cinq tiers.** Le corpus `tests/fixtures/arch_paraphrase_texts/` (17 cas) est étendu aux tiers 0, 0b, 1a et 1b, chacun avec au moins un cas positif et un cas négatif. Le corpus inclut le marqueur cité au milieu d'une ligne, qui ne doit pas supprimer la disposition, et un `READY` cité en ligne avant une escalade du moteur, où l'escalade doit gagner. Le bash d'avant la bascule (`pre-switch`) et la sous-commande rendent les mêmes jetons sur tout le corpus, avec un terme d'anti-vacuité (§ 4) : un `dispatch-lib.sh` qui ne se source plus rougit.
- **AC2 : non-régression `post-switch`.** Les tests existants de `test-dispatch-lib.sh` et de `_shared/tests/` qui appellent `_parse_disposition`, `_parse_verdict` ou `_disposition_was_fuzzy` passent sans modification de leurs attentes. Les dix appels directs de `_parse_disposition_fuzzy` et `_parse_verdict_fuzzy` dans les tests (mesurés le 2026-11-02) ne disparaissent pas avec ces fonctions : chacun devient un cas du corpus doré, avec son attente inchangée.
- **AC3 : code `≥2`.** Une sous-commande absente ou un fichier illisible produit un refus nommé dans le `RESULT`, jamais un UNPARSED silencieux. Un test épingle les deux polarités dans le même corps : le refus sort quand le moteur est absent, et rien ne sort sur une réponse saine.
- **AC4 : le scan anti-copie (§ 5).** Aucun motif de tier 1 ou 2 ne subsiste dans `dispatch-lib.sh`. Le scan est rouge si une table revient.
- **AC5 : la famille de sous-commandes.** Tranché au vol 4 (§ 7 du plan) : la famille reste plate, et le critère du § 6 est remplacé. Prime l'a accepté le 2026-11-02. La doctrine reçoit la décision par une PR séparée, suivie ici, et **non** par le vol 5.
- **AC6 : la doctrine est mise à jour.** Les § 6 et 7 de `dispatch-lib-migration.md` passent la phase 3 à « livrée » et notent la décision sur le canal ci-dessus.

### État de chaque critère à la sortie de ce vol

| critère | état | où |
|---|---|---|
| AC1 | **déjà livré** par `593a745c5` (vol 4). Ce vol en retourne le statut de preuve en non-régression et en dit la raison. | R5, R8, § 1 |
| AC2 | **livré ici**, y compris la clause CI que le corps ne prévoyait pas (§ 4). La clause des « dix appels directs » est close par construction depuis le vol 4 (§ 1-i). | R4, R5, R6, R7 |
| AC3 | **livré ici**, aux deux polarités, avec le canal de refus qui traverse le `$(...)` (§ 3). | R1, R4 |
| AC4 | **partiel ici** (absence des littéraux discriminants de tier 2), **tier 1 au vol 6** — chiffré et tracké. | R5, § 6 |
| AC5 | **clos** par `45af2b338` (#3015). Non rouvert. | § 1 |
| AC6 | **au vol 6**, avec AC4 : la doctrine ne déclare pas livré ce qui ne l'est pas. | § 6, R10 |
