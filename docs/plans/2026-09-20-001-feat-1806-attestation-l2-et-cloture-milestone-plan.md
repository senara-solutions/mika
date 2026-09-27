# mika#1806 — l'attestation réseau cesse d'être une variable d'environnement, et le milestone devient clôturable

**Ticket :** [mika#1806](https://github.com/senara-solutions/mika/issues/1806) — backend de recherche web derrière un egress contrôlé, no-log / minimise-et-délie
**Type :** feat (substrat d'audit) + docs (clôture)
**Priorité :** p1-important

---

## Le premier livrable du grooming : l'état mesuré

Le ticket a été dé-parqué le 20/09 **par épuisement du stock vivant**, pas parce que
quelqu'un avait établi qu'il restait du travail (commentaire opérateur, mot pour mot :
« Motif : stock vivant épuisé »). Établir l'état était donc le préalable, et il déplace
le périmètre : **cinq des six sous-issues sont livrées et mergées.**

| | sous-issue | état mesuré dans l'arbre | preuve |
|---|---|---|---|
| **E1** | egress substrate, point de sortie unique | **livré** | `crates/mika-gateway/src/egress_search/mod.rs` (753 l.), `SearchEgressClient` `pub(crate)` non convertible ; lint CI `egress-uniqueness-lint` → `scripts/verify-egress-uniqueness.sh`. #1807 → PR #1909 |
| **E2** | client Brave derrière l'egress | **livré** | `egress_search/brave.rs` (727 l.), `MIKA_SEARCH_UPSTREAM` / `MIKA_BRAVE_API_KEY` sur le gateway. #1808 → PR #1911 |
| **E3** | dé-liage identité ↔ requête | **livré** | `tests_e3_request_shape.rs` (349 l.), lint CI `egress-request-shape-lint`, Q4 STRIP TOTAL. #1809 → PR #1912 |
| **E4** | no-log vérifié end-to-end | **livré côté couches 1 et 3** | `tests_e4_no_log.rs`, `verify-egress-no-log.sh` + son contrôle négatif (mika#2054), runbook trois-couches (319 l.). #1810 |
| **E5** | déblocage clé pour le testeur | **moitié mika livrée** (mika#2407 : garde de démarrage `assert_search_substrate_expectation`, `MIKA_SEARCH_REQUIRED`, `scripts/smoke-search-substrate`) ; **moitié provisionnement hors dépôt** | #1811 |
| **E6** | SearXNG, design-only | **livré** | `docs/egress-search-searxng-contingency.md` (241 l.), avec ses conditions d'escalade. #1812 |

**Conséquence sur le périmètre : il ne reste aucun code de substrat à écrire.** Ce qui
reste ouvert est la **propriété 4** du corps du ticket — *« Vérifiable : n'importe qui
peut auditer par lecture du code + réseau qu'il n'y a ni rétention ni liage »* — et la
condition de clôture du milestone lui-même. Un plan qui re-livrerait E1–E4 serait du
travail fabriqué ; c'est le premier risque de ce ticket et il est écarté ici par écrit.

---

## Le défaut, lu à la ligne

La couche 2 du runbook (métadonnées réseau : iptables/nft, logs de proxy, VPC flow
logs) est **SPEC-only** par décision datée : son implémentation vit dans `mika-cloud`.
Le contrôle qui la tient côté mika est donc une **attestation hors-bande de
l'opérateur**, et cette attestation est aujourd'hui une variable d'environnement :

```
scripts/audit-egress-no-log.sh:40   SUPPRESS_L2_WARN="${MIKA_AUDIT_SUPPRESS_L2_WARN:-0}"
scripts/audit-egress-no-log.sh:190      echo "  NOTE: Layer 2 warning suppressed via MIKA_AUDIT_SUPPRESS_L2_WARN=1."
scripts/audit-egress-no-log.sh:295   echo "  PASS: all layers clean (Layer 2 confirmed via MIKA_AUDIT_SUPPRESS_L2_WARN)."
```

Le runbook la qualifie de *« the operator's signed statement »*. Elle n'est signée de
rien : elle ne porte **ni instant, ni cible, ni auteur, ni péremption**. Posée une fois
dans un cron, un `EnvironmentFile` ou un profil shell, elle fait imprimer *« all layers
clean »* à chaque exécution suivante, pour toujours, sans qu'aucune vérification ait eu
lieu. **« attesté à l'instant sur ce cluster » et « attesté il y a six mois sur un autre »
produisent les mêmes octets** — c'est la forme exacte de l'asymétrie que mika#2407 a dû
nommer pour `MIKA_SEARCH_REQUIRED`, et le mode de panne que mika#2293 résume : *un
réglage qu'on ne peut pas observer n'est pas un réglage, c'est un espoir.*

**Le contrôle positif est daté et vécu.** La rotation vers l'image `main-e1342dfa`
(2026-09-18) a déplacé la recherche derrière le gateway et fait perdre la recherche web
à six tenants pendant ~20 h. Une rotation change le chemin réseau ; une attestation qui
ne nomme pas le déploiement qu'elle couvre survit à la rotation en silence et continue
de dire « clean » à propos d'une topologie qui n'existe plus.

**Périmètre du défaut, dit honnêtement :** aucun consommateur actuel ne pose cette
variable (`grep` sur l'arbre : uniquement le script et ses trois docs). Le défaut est
donc **latent**, pas actif. Il est traité maintenant parce que la variable est le seul
chemin documenté vers `exit 0`, et que la doctrine du milestone conditionne le câblage
de toute clé API à une vérification no-log **de construction** — un contrôle qui rend
« vert » sur une variable d'environnement ne peut pas porter cette garantie.

---

## Livrables

### L1 — L'attestation couche 2 devient un enregistrement daté, ciblé et périssable

**Écriture.** `scripts/audit-egress-no-log.sh --attest --target <cible> --by <auteur>`
écrit `~/.mika/state/egress-l2-attestation.json` (chemin surchargeable par
`MIKA_EGRESS_L2_ATTESTATION_FILE` pour les tests) :

```json
{
  "attested_at": "2026-09-20T09:14:03Z",
  "target": "mika-cloud/prd/gateway",
  "attested_by": "samidarko",
  "ttl_days": 30
}
```

`~/.mika/state/` est le répertoire maison de cet état — précédents `pr-origin-epoch`
(mika#2026), `auto-pull-stop` (mika#2329), `pilot-gitconfig`. Un fichier, parce que le
script doit fonctionner pendant un incident, côté gateway, sans base joignable.

`--target` et `--by` sont **obligatoires**. La durée est le filet ; **la cible est le
contrôle** : le discriminant honnête d'une attestation réseau n'est pas son âge mais la
topologie qu'elle a couverte.

**Lecture, fail-closed sur chaque terme.** Le même script est le lecteur unique :

| état lu | verdict | sortie |
|---|---|---|
| fichier absent | couche 2 due | `exit 2` — l'état actuel de tout le monde, donc zéro régression |
| JSON illisible ou malformé | couche 2 due | `exit 2` en le disant — une attestation illisible n'est pas une attestation |
| `attested_at` absent ou non-parsable | couche 2 due | `exit 2` |
| périmé (`now > attested_at + ttl`) | couche 2 due | `exit 2`, en nommant l'âge et l'instant |
| `target` ≠ `MIKA_EGRESS_L2_TARGET` (quand posée) | couche 2 due | `exit 2`, en **nommant les deux** valeurs |
| sinon | couche 2 attestée | `exit 0`, et la ligne de PASS **nomme l'instant, la cible et l'auteur** |

Ce dernier point est la moitié qui compte : un `PASS` ne doit plus pouvoir être lu sans
savoir sur quoi il porte.

**`MIKA_AUDIT_SUPPRESS_L2_WARN` est retirée, et son retrait est dit.** Si elle est
encore posée, le script émet
`WARN: MIKA_AUDIT_SUPPRESS_L2_WARN is set and no longer suppresses anything — record an
attestation with --attest instead` et **ne descend pas l'exit**. Motif : un opérateur
qui croit avoir attesté et ne l'a pas fait doit l'apprendre, pas le découvrir sur un
incident — même raisonnement et même forme que `auto_pull_stop_stale_env_knob`
(mika#2329). Un simple ignore silencieux reproduirait le défaut d'un cran plus bas.

**TTL.** `MIKA_EGRESS_L2_ATTESTATION_TTL_DAYS`, défaut **30**, trois paliers maison :
absente/vide → défaut ; illisible, `0` ou négative → défaut **avec un WARN nommant la
valeur entre guillemets**. Le `0` ne désarme pas : sur un contrôle doctrinal, une faute
de frappe qui rendrait l'attestation éternelle serait la panne que tout ceci ferme.
30 jours parce qu'une attestation porte sur une topologie qui bouge aux rotations ;
pas moins, parce qu'un TTL qui périme plus vite que la cadence de déploiement transforme
le contrôle en formalité qu'on renouvelle sans regarder.

### L2 — Un point d'entrée unique, qui répond par propriété et non par script

Aujourd'hui, auditer le milestone suppose de savoir que cinq scripts existent
(`verify-egress-uniqueness.sh`, `verify-egress-no-log.sh`, `test-verify-egress-no-log.sh`,
`verify-egress-request-shape.sh`, `audit-egress-no-log.sh`) et de lire ~1 100 lignes de
documentation pour savoir lequel tient quoi. La propriété 4 dit *« n'importe qui peut
auditer »* : elle n'est pas tenue tant que l'entrée n'est pas trouvable.

`make audit-egress` (→ `scripts/audit-egress`) agrège les contrôles existants — il n'en
réécrit **aucun** — et rend un verdict **par propriété du corps du ticket** :

```
Propriété 1 — un seul point de sortie      PASS   (verify-egress-uniqueness.sh)
Propriété 2 — zéro rétention               OWED   (couches 1+3 PASS ; couche 2 non attestée)
Propriété 3 — dé-liage identité ↔ requête  PASS   (verify-egress-request-shape.sh + Q4)
Propriété 4 — vérifiable                   PASS   (cette sortie)
```

`exit 0` seulement si les quatre sont `PASS`. `smoke-search-substrate` exige un gateway
joignable : il est **optionnel** et rend `SKIP` en l'absence de cible, jamais `PASS` —
un contrôle qui n'a rien vérifié ne compte pas pour un vert (règle d'`exit 2` de
`smoke-search-substrate`, mika#2407).

### L3 — L'état de clôture est écrit là où la prochaine personne le cherchera

Le tableau d'état ci-dessus, son critère de fermeture et les deux moitiés hors dépôt
sont portés dans `crates/mika-gateway/docs/egress-search.md` (§ *État du milestone
#1806*) et résumés dans `crates/mika-gateway/CLAUDE.md` § *Search Substrate*. Sans cela,
la prochaine personne qui dé-parque #1806 refera l'enquête sur cinq scripts et quatre
documents — l'enquête que ce grooming vient de faire.

---

## Ce qui n'est PAS livrable ici, et pourquoi

- **L'implémentation de la couche 2** (règles iptables/nft, NetworkPolicy K8s, config
  Envoy/HAProxy). Elle vit dans `mika-cloud`, **absent de ce workspace**. Décision
  déjà datée dans le runbook (§ Layer 2, *« What this ticket does NOT deliver »*). Ce
  plan livre le contrôle qui la **rend attestable et périssable**, jamais la règle.
- **E5, le provisionnement effectif de la clé.** Geste opérationnel dans `mika-cloud`,
  et le guardrail du ticket est explicite : *« Aucun deploy prod sans mains de
  Vincent. »* La moitié mika (garde de démarrage + smoke externe) est déjà livrée par
  mika#2407.
- **Le backoff 429 sur le quota partagé Brave.** Découpé en p2 par l'opérateur sur
  mika#2407 ; durcissement réel, mais ce n'est pas la propriété doctrinale de #1806.
- **Toute modification de `egress_search/*.rs`.** Le substrat est tenu par trois lints
  CI ; le toucher ici serait un risque sans contrepartie.

---

## Risques et contre-mesures

| risque | contre-mesure |
|---|---|
| **Travail fabriqué** — re-livrer E1–E4 | Périmètre écrit ci-dessus ; aucun fichier de `egress_search/` n'est touché ; le diff se limite à `scripts/`, `Makefile` et trois documents |
| **Le nouveau chemin casse un consommateur** de `MIKA_AUDIT_SUPPRESS_L2_WARN` | `grep` sur l'arbre : zéro consommateur hors du script et de ses docs. Le retrait est néanmoins **dit** (WARN) plutôt que silencieux |
| **L'agrégateur devient un second lecteur** des contrôles, qui dérive | `scripts/audit-egress` **invoque** les scripts existants et ne duplique aucun prédicat ; il traduit des codes de sortie en propriétés |
| **Un `exit 0` obtenu par accident** (fichier d'attestation bricolé) | Tous les termes sont fail-closed et le `PASS` **imprime** l'instant, la cible et l'auteur : un vert non mérité est lisible dans sa propre sortie |

---

## Definition of Done

- `scripts/audit-egress-no-log.sh` ne rend `exit 0` que sur un enregistrement
  d'attestation valide, non périmé et de cible concordante ; aucune variable
  d'environnement ne peut plus produire ce résultat.
- `--attest --target <t> --by <who>` écrit l'enregistrement de façon atomique et refuse
  une invocation dont un des deux arguments manque.
- `make audit-egress` existe, agrège les contrôles sans en dupliquer un seul, et rend
  un verdict par propriété du ticket.
- Les trois documents (`egress-search.md`, `egress-search-no-log-audit.md`,
  `mika-gateway/CLAUDE.md`) décrivent le geste d'attestation et l'état du milestone ;
  aucune occurrence de `MIKA_AUDIT_SUPPRESS_L2_WARN` ne subsiste comme chemin
  recommandé.
- `cargo test`, `cargo clippy`, `make verify-egress-no-log` verts ; les trois jobs CI
  egress inchangés et verts.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de ses quatre propriétés à blinder et des livrables ci-dessus.

- **AC1 — Une attestation absente n'est jamais un vert.** Sans fichier d'attestation,
  `scripts/audit-egress-no-log.sh` rend `exit 2` et nomme le geste qui manque, même si
  `MIKA_AUDIT_SUPPRESS_L2_WARN=1` est posée dans l'environnement.
- **AC2 — Une attestation périmée n'est jamais un vert.** Un enregistrement dont
  `attested_at` est antérieur à `now − ttl` rend `exit 2` en nommant l'âge mesuré et
  l'instant attesté.
- **AC3 — Une attestation d'une autre cible n'est jamais un vert.** Avec
  `MIKA_EGRESS_L2_TARGET` posée et différente du champ `target`, le script rend
  `exit 2` en **nommant les deux valeurs**.
- **AC4 — Une attestation illisible n'est jamais un vert.** Fichier vide, JSON
  malformé, `attested_at` absent ou non-parsable : `exit 2` dans chaque cas, jamais
  `exit 0` et jamais un plantage non diagnostiqué.
- **AC5 — Un vert porte sa provenance.** Sur attestation valide, `exit 0` et la ligne
  de PASS contient l'instant, la cible et l'auteur attestés.
- **AC6 — Le geste d'attestation est complet ou refusé.** `--attest` sans `--target`
  ou sans `--by` refuse d'écrire et rend un code non nul ; une écriture réussie produit
  un JSON relisible par le lecteur du même script (aller-retour vérifié).
- **AC7 — Le TTL suit les trois paliers maison.** Absent → 30 ; illisible, `0` ou
  négatif → 30 **avec un WARN nommant la valeur entre guillemets** ; valide → honoré.
- **AC8 — La variable retirée est dite, pas ignorée.** `MIKA_AUDIT_SUPPRESS_L2_WARN=1`
  posée produit un WARN nommant le geste qui marche, et ne change aucun code de sortie.
- **AC9 — Le point d'entrée unique répond par propriété.** `make audit-egress` rend une
  ligne par propriété (1 à 4) avec le contrôle qui la tient, et `exit 0` seulement si
  les quatre sont `PASS` ; un contrôle non exécutable rend `SKIP`, jamais `PASS`.
- **AC10 — Aucun contrôle n'est dupliqué.** L'agrégateur n'introduit aucun prédicat
  d'analyse propre : chaque verdict provient du code de sortie d'un script existant.
- **AC11 — Le substrat n'est pas touché.** Le diff ne modifie aucun fichier de
  `crates/mika-gateway/src/egress_search/` ; les trois lints CI egress restent verts.
- **AC12 — L'état du milestone est écrit.** Les six sous-issues, leur état et les deux
  moitiés hors dépôt (couche 2, provisionnement E5) sont lisibles depuis la
  documentation du gateway sans relire les scripts.

## Vérification

- **Tests de script** — `scripts/test-audit-egress-attestation.sh`, sur le modèle
  existant de `scripts/test-verify-egress-no-log.sh` (contrôle négatif du lint,
  mika#2054) : un cas par terme fail-closed d'AC1–AC7, chacun sur un fichier
  d'attestation fabriqué dans un répertoire temporaire via
  `MIKA_EGRESS_L2_ATTESTATION_FILE`.
- **Contrôle de bonne foi** — un cas assertant qu'une attestation **valide** rend bien
  `exit 0`, pour que « tous les cas rendent 2 » soit distinguable de « le script est
  cassé » (mika#2205 : un contrôle silencieusement inactif se lit comme un contrôle qui
  n'a rien trouvé).
- **CI** — le nouveau test rejoint le job `egress-no-log-lint`, à côté de
  `test-verify-egress-no-log.sh` ; pas de nouveau job.

## Sonde post-déploiement, et ses haltes

1. Lancer `make audit-egress` sur le poste opérateur : attendu `Propriété 2 — OWED`
   (aucune attestation n'existe encore) et un `exit` non nul. **Un `PASS` sur la
   propriété 2 au premier lancement est une halte** : il signifie qu'un fichier
   d'attestation préexiste ou qu'un terme n'est pas fail-closed.
2. Attester (`--attest --target … --by …`), relancer : attendu quatre `PASS`, `exit 0`,
   et la provenance imprimée sur la ligne de la propriété 2.
3. Avancer l'horloge au-delà du TTL (ou éditer `attested_at`) : attendu retour à `OWED`.
   **Si le vert survit à la péremption, halte** — ne pas rallonger le TTL par réflexe,
   c'est le prédicat de lecture qui est en cause.
4. **Halte générale :** si un `exit 0` est obtenu sans qu'aucune attestation ait été
   enregistrée, **ne pas ajuster la sortie** — c'est le chemin d'une variable
   d'environnement qui subsiste quelque part, et c'est lui qu'il faut retirer.

## Rollback

Trois fichiers de scripts, un `Makefile` et trois documents. `git revert` de la PR
restaure intégralement le comportement précédent (couche 2 attestée par variable
d'environnement). Aucune migration, aucun schéma, aucun changement de code Rust, aucun
format de fil consommé par un tiers : le fichier d'attestation laissé sur disque devient
simplement inerte.
