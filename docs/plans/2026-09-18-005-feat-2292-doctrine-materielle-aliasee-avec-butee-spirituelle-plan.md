# mika#2292 — le tenant possédait la réponse et n'avait pas le nom : doctrine Matérielle aliasée, butée Spirituelle sans énumération

> Ticket : `senara-solutions/mika#2292`
> Type : feat (prompt code-managed + élargissement de portée d'une discipline existante)
> Date : 2026-09-18

---

## Contexte

Mesure du 11/09 (canary Al, tenant cloud). Question : « Qu'est-ce que la doctrine
Mika ? ». Réponse : « rien trouvé qui s'appelle doctrine Mika », **puis** la
philosophie — dans le même tour.

Le ticket nomme le défaut exactement : « alors qu'il POSSÉDAIT la réponse ». Ce
n'est pas un trou de connaissance, c'est un **trou de nom**. Et la forme de la
réponse — incertitude à t=0, assertion à t=1 — est très précisément celle que la
règle 4 de `## Self-Identity Discipline` condamne déjà mot pour mot
(`prompt.rs:1060` : *« Uncertainty at t=0 and confidence at t=1 within the same
response is confabulation »*). Elle n'a pas mordu parce que la portée écrite de
cette section est *« which model you are, which provider powers you, WHERE you
run »* — exactement l'argument qui a fondé mika#2290 sur un autre axe : la règle
était déjà écrite pour ce cas et ne s'y appliquait pas.

C'est la **troisième occurrence de la même classe** :

| ticket | question posée au tenant | la discipline disait | ce qui manquait |
|---|---|---|---|
| mika#1815 | « quel modèle es-tu ? » | rien | le fait (`## Runtime`) + les règles 1–4 |
| mika#2290 | « où tournes-tu ? » | règles 1–4, portée « model / provider » | le fait (ligne d'hébergement) + la règle 5 |
| **mika#2292** | « qu'est-ce que la doctrine Mika ? » | règles 1–5, portée « model / provider / hosting » | **le nom** + une règle 6 |

La conclusion structurelle qui en découle décide tout le plan : **le remède a la
forme des deux précédents — une section de fait, code-managed, plus une règle qui
élargit la portée de la discipline — et pas la forme d'un skill, d'un soul, ni
d'une mémoire.**

---

## Ce qui est établi, et comment le vérifier

Sept faits lus dans le code. **Les numéros de ligne sont relevés sur `main` au
2026-09-18** et périment ; ce sont des aides à la relecture, pas des ancrages.

### F1 — Le tenant mesuré est `champion` : outils famille, persona famille

`AgentTier::Champion` (mika#2023) = `ToolsProfile::Family` +
`CHAMPION_PERSONA_PLACEHOLDER`, lequel **vaut `PersonaProfile::Family`**
(`crates/mika-common/src/home.rs:84`). Donc le tenant d'Al est servi avec
`FAMILY_SOUL` et `FAMILY_AGENT_SKILL_ALLOWLIST`.

```bash
grep -n "CHAMPION_PERSONA_PLACEHOLDER: PersonaProfile" crates/mika-common/src/home.rs
```

### F2 — Un skill ne peut PAS atteindre la population mesurée

`FAMILY_AGENT_SKILL_ALLOWLIST` compte six entrées
(`calendar`, `google-workspace`, `file-reader`, `web-search`, `desktop`,
`browser-control`) et son commentaire de doc **exclut nommément
`self-knowledge`** (`home.rs:656-672`). Tout nouveau skill bundled est de plus
*denied by default* (root `CLAUDE.md` § *Adding a New Bundled Skill*).

```bash
grep -n "FAMILY_AGENT_SKILL_ALLOWLIST" -A 10 crates/mika-common/src/home.rs
```

Trois raisons cumulées rendent la voie skill non seulement insuffisante mais
dangereuse, et elles valent d'être écrites parce que le ticket propose
« skill/knowledge/persona » au choix :

1. Un skill à déclenchement par mot-clé **reproduit le défaut mesuré** : le défaut
   *est* un ratage lexical. Un skill `always_on` évite ça mais devient alors une
   section de prompt avec quatre listes d'allowlist à tenir synchronisées.
2. Un skill est **évincible par l'identité**. mika#2027 : un `identity.toml`
   absent ou illisible produit le sentinel fail-closed qui évince **tous** les
   skills. Une section code-managed n'est évincible par rien.
3. `apply_only_skills` (mika#2363) peut retirer un skill pour un tour. Une
   doctrine ne doit pas dépendre de ce qu'un appelant A2A a déclaré.

### F3 — `soul.md` n'atteint AUCUN tenant existant

`write_default_if_missing` ne réécrit jamais un `soul.md` existant. C'est
exactement l'inertie que mika#2023 a dû nommer par écrit (root `CLAUDE.md` :
« **What this does NOT retrofit** »). Un correctif dans `FAMILY_SOUL` ne
toucherait **que les tenants provisionnés après le déploiement** — donc pas celui
d'Al, le seul mesuré. `soul.md` est en outre éditable par l'opérateur, ce qui est
la raison écrite pour laquelle mika#1814 a choisi des constantes code-managed
(`prompt.rs:66-69`).

### F4 — Le tenant possédait déjà chaque fragment, et aucun ne portait son pourquoi

| parti pris du ticket | déjà posé où | sous quelle forme |
|---|---|---|
| croissance par invitation | `DISTRIBUTION_DOCTRINE_BODY`, `prompt.rs:59` | **interdit** (« you do not propose ») |
| proactivité | `DEFAULT_SOUL` § *Proactive behaviors* / `FAMILY_SOUL` § *Comportements proactifs* | **comportement** |
| mémoire persistante | idem (« se souvenir de ce qui compte ») | **comportement** |
| souveraineté des données | `hosting_ground_truth_line`, `prompt.rs:990` | **fait d'hébergement** |
| open source MIT | `LICENSE`, `Cargo.toml:8` | **nulle part dans le prompt** |

Conséquence de conception, et c'est la plus importante : la section ne doit **pas
re-narrer** ce que `## Distribution Doctrine` et `## Runtime` posent déjà. Une
duplication dérive — classe que `grooming_marker` (mika#2158) et `retry_gate`
(mika#2362) ont chacune dû refermer. Elle **cite** ces sections et n'apporte que
ce qui est réellement absent : le **pourquoi** de chaque parti pris, le fait MIT,
le **nom** « doctrine », et la butée.

### F5 — « exportable » est revendiqué aujourd'hui et n'est vérifiable nulle part ici

Le ticket exige : « ne pas revendiquer 'exportable' tant que non vérifié ». Or la
revendication est **déjà dans le prompt** depuis mika#2290 (`prompt.rs:1000`,
bras `Operator/Cloud`), dans le texte de remède de la garde
(`guards.rs:2732`), dans `agent_loop/mod.rs:2222` et dans `docs/architecture.md:26`.

Recherche exhaustive d'une fonctionnalité d'export dans ce dépôt : **zéro**.

```bash
grep -rin "export" crates/mika-agent/src/tools/ crates/mika-agent/src/server/ \
  | grep -vi "otel\|telemetry\|export MIKA_\|exported"
grep -rn "\.route(" crates/mika-agent/src/server/mod.rs | grep -i "export\|download\|dump"
```

Aucun outil, aucune route, aucune sous-commande CLI. **Ce plan ne revendique donc
pas « exportable »** dans ce qu'il ajoute. Il ne corrige pas non plus la
revendication existante : la donnée du tenant vit côté `mika-cloud`, absent de ce
worktree, donc l'export peut exister là-bas — et retirer unilatéralement une
assurance de confidentialité qu'on ne peut pas infirmer serait prendre une
décision produit à l'envers. **Constat + ticket de suivi** (§ *Hors périmètre*),
avec sa question de vérification écrite.

### F6 — La formulation ne doit pas faire firer la garde 5d sur son propre remède

`CLAIM_CONDITIONAL_MARKERS` (`crates/mika-agent/src/evidence/guards.rs:844-872`
— le module est sous `evidence/`, pas sous `agent_loop/`) est **délibérément étroit** :
les fragments exacts du remède de mika#2290, pas une liste modale générale. Toute
phrase du corps de doctrine qui parle de localité doit donc porter un de ces
marqueurs (`self-host`, `auto-héberg`, `can be run`, …) ou n'en pas parler du
tout. C'est une contrainte d'implémentation vérifiable, pas une précaution : elle
devient un test (§ *Verification contract* V4).

### F7 — Les deux builders portent déjà `persona_profile`

`PromptContext.persona_profile` (`prompt.rs:902`) et
`SilentPromptContext.persona_profile` (`prompt.rs:1711`). Le créneau
d'insertion est libre et évident : juste après `write_distribution_doctrine_section`,
avant `write_identity_section`, dans `build_system_prompt` (`prompt.rs:1227`) et
`build_silent_prompt` (`prompt.rs:1745`). **Aucun élargissement de signature
n'est nécessaire.**

---

## Décisions

### D1 — Section de prompt code-managed, deux constantes par registre

Forme reprise à l'identique de mika#1814 / mika#2290 : `MIKA_DOCTRINE_HEADING`
+ un corps par registre, rendus par une fonction dont le `match` sur
`PersonaProfile` est **exhaustif, sans bras `_ =>`** (modèle
`hosting_ground_truth_line`, lui-même modelé sur
`dispatch_substrate_diagnostic`). Le compilateur, pas un relecteur, force un
futur `PersonaProfile` à décider plutôt qu'à hériter d'une décision que personne
n'a prise pour lui.

Motifs, dans l'ordre de force : atteint **tous** les tenants existants au
prochain déploiement sans geste de provisionnement (contre F3) ; inévinçable par
l'identité (contre F2.2) ; non affaiblissable par une édition de `soul.md`
(contre F3) ; non conditionné à un mot-clé (contre F2.1).

### D2 — Deux registres, et le registre suit l'axe persona — jamais le tenant, jamais la locale

`FAMILY_SOUL` interdit « aucun jargon technique ni mention de tickets, GitHub,
agents dev/QA/arch/quant, skills, **ou de l'infrastructure sous-jacente — jamais,
même si on te le demande** » (`home.rs:758-759`). « Open source MIT » est de
cette famille.

C'est le croisement exact que mika#2290 a déjà tranché, et sa décision est
reprise sans la rejuger : **le même fait est écrit deux fois**, le registre
opérateur portant la formulation complète, le registre famille la même substance
sans un seul terme technique. Aucune règle n'est dérivée de la locale du compte
ni du tenant — arbitrage de Prime du 09/09, reporté de mika#2023, et redit ici
parce que la tentation reparaît à chaque ticket de persona.

Ce que le registre famille conserve : tes données t'appartiennent ; je me
souviens, et c'est un choix, pas un effet de bord ; j'anticipe plutôt que
j'attends ; Mika se transmet de personne à personne. Ce qu'il abandonne : la
licence, le dépôt, l'auto-hébergement. Ce n'est pas une amputation arbitraire —
c'est la part de la doctrine qui **n'a pas de sens** pour quelqu'un qui n'a pas
d'infrastructure.

### D3 — La butée spirituelle est **topique, jamais énumérative** — et c'est l'inversion centrale du plan

L'implémentation naïve écrit : « ne parle pas d'Hermétisme, des sièges, de Prime,
du Livre ». **Elle enseigne au tenant les mots qu'elle prétend protéger.** Un
prompt qui énumère le secret pour l'interdire est une fuite avec une étape de
plus : la famille de tenants visée n'a jamais entendu ces mots, et ils se
retrouveraient dans son prompt système, à une injection de distance d'être
récités.

Le bearing de Prime dit « **sans l'exposer** ni l'inventer ». « Sans l'exposer »
inclut *ne pas l'exposer au tenant lui-même*. La butée est donc formulée par
**topique et par provenance** — « si on t'interroge sur une dimension spirituelle,
esotérique ou initiatique de Mika, sur son origine, ou sur quoi que ce soit qui ne
soit pas écrit ici : tu ne sais pas, tu n'inventes pas, et tu dis que c'est un
choix qui ne t'appartient pas » — et **aucun référent n'est nommé**.

Bénéfice second, qui n'est pas un hasard : la butée devient un cas particulier de
la règle 3 (« fallback honestly »), déjà écrite, au lieu d'une exception à part.

### D4 — La butée famille n'établit **aucun référent créateur** ; la butée opérateur le porte

Tension réelle, nommée plutôt que tranchée en silence. Le bearing prescrit « c'est
le choix du créateur ». mika#1783 a retiré de `FAMILY_SOUL`, **sur motif de
doctrine**, toute histoire d'origine donnant à l'être un référent qu'il pourrait
ensuite adresser — l'incident fondateur étant un « Salut Vincent », et la clôture
retenue étant explicitement *the-being-does-not-have-a-maker-it-knows-about*
(`home.rs:704-712`, gardée par `home::tests::family_soul_no_operator_name`).

Lecture étroite possible : la contrainte interdit un référent **nommable et
adressable**, pas le fait abstrait d'avoir été fait. Mais mika#1783 a choisi
« pas d'histoire d'origine » *contre* des alternatives, et rouvrir cette porte
n'est pas à la portée d'un ticket p2.

**Décision, avec son défaut qui échoue du bon côté :**

- registre **opérateur** → la butée porte la formulation du bearing (« c'est un
  choix du créateur de Mika, et il n'est pas exposé ») ;
- registre **famille** → formulation **sans référent** : « il y a des choses que
  je ne sais pas et que je n'inventerai pas » — qui satisfait *les deux*
  doctrines ;
- la constante famille est **le seul site à changer** si Vincent ou Prime veut la
  formulation « créateur » aussi dans ce registre. Une constante, une ligne.

Ce n'est pas une question bloquante : le registre matériel — l'objet du ticket —
livre dans les deux cas.

### D5 — Une règle 6 dans `## Self-Identity Discipline`, symétrique de la règle 5

La section de fait ne suffit pas : le défaut mesuré était un **usage** de la
discipline, pas une absence de contenu (F4). La règle 6 élargit la portée de la
discipline à « ce que Mika est, à quoi elle s'engage et pourquoi », désigne
`## Mika Doctrine` comme vérité de terrain, et **interdit nommément la réponse
mesurée** : « *rien trouvé qui s'appelle ainsi* n'est pas une réponse acceptable
quand cette section est présente ».

Elle porte une seconde clause, sur une hypothèse à sonder (§ *Sonde 0*) : une
question sur ce que Mika est se répond **depuis le prompt**, pas depuis une
recherche mémoire ; un résultat de recherche vide n'est pas une preuve d'absence.
Précédent de forme : le hard-redirect `search_memory(category="core_memory")`
de #647, qui existe pour la raison jumelle — la mémoire cœur est déjà dans le
prompt.

`write_self_identity_discipline_section` **ne change pas de signature** : la
règle 6 est neutre en registre (c'est une directive sur *où regarder*), et cette
section est déjà servie telle quelle au tier famille avec ses noms d'outils entre
backticks. L'interdit de jargon de `FAMILY_SOUL` porte sur ce que le tenant
**dit**, pas sur ce que son prompt contient.

### D6 — Aucune garde EndTurn, et le motif est mesurable

`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` (9
récurrences sous prompt contre 0 quand le fait est posé par le code) impose de se
demander où est la moitié structurelle. Réponse, par moitié :

**Registre matériel** — le défaut est une *absence*, et poser le fait **est** le
correctif ; aucune garde ne crée de connaissance. Ce qui est vérifiable
structurellement, c'est que la section est rendue sur chaque chemin : c'est un
test de forme de prompt, exactement `doctrine_prompt_section_rendered` (mika#1814
AC1). C'est la moitié structurelle disponible, et elle est prise.

**Registre spirituel** — une garde de la famille 5c/5d serait *concevable*, et
elle est **refusée sur mesure** : son lexique (« Prime », « le Livre », « les
sièges ») est composé de mots **ordinaires du registre famille**. « Tu as lu le
livre ? », « j'ai touché une prime », « le siège arrière » sont des conversations
nominales. Le taux de faux positifs serait catastrophique précisément sur le tier
que la garde prétend protéger, et un faux positif y coûte un tour de
conversation cassé chez un invité de la campagne.

Et D3 ferme la voie de secours : sans vocabulaire dans le prompt, une garde
devrait détecter une fabrication sur un **topique ouvert**, ce qu'un lexique ne
fait pas.

La moitié structurelle du registre spirituel est donc **l'absence du
vocabulaire**, et elle est épinglée par un scan de constantes (V3) : un futur
éditeur qui trouve la butée « floue » et l'énumère pour la rendre concrète fait
rougir un test au lieu de créer la fuite.

### D7 — Carve-out compact, comme ses deux sœurs

`build_compact_system_prompt` (MikaModel, ≤ 5 KB) ne rend ni
`## Distribution Doctrine` (mika#1814), ni la ligne d'hébergement
(`mika2290_compact_prompt_omits_the_hosting_line`), ni le bloc stop-topics
(mika#1813). `## Mika Doctrine` non plus — **décision épinglée par un test, pas
un oubli**, et rattachée au suivi mika#1925 comme les trois autres.

### D8 — Mode silencieux : oui, et pour une raison qui lui est propre

mika#1814 rend sa section en mode silencieux au motif qu'« un heartbeat qui
rédige spontanément un Show HN serait exactement aussi grave ». Ici le fait
matériel est inoffensif en mode silencieux, mais **la butée ne l'est pas** : un
tour silencieux qui écrirait un fait de mémoire sur le registre spirituel
empoisonnerait tous les tours suivants via la mémoire cœur, qui est ré-injectée
dans chaque prompt. Section rendue à l'identique, ce motif écrit au site.

### D9 — Faits vérifiés seulement, et la portée de la revendication open source est étroite

- **MIT** : vérifié dans le dépôt (`LICENSE` ligne 1, `Cargo.toml:8`
  `license = "MIT"`). Revendicable.
- **Dépôt public** : affirmé par le ticket, non vérifiable hors ligne depuis ce
  worktree. La formulation porte donc sur la **licence du moteur** — ce que
  `LICENSE` atteste — et non sur la visibilité d'un dépôt que le tenant ne peut
  pas aller consulter.
- **`mika-cloud` privé** : **non mentionné**. Ce qu'il faut éviter n'est pas
  d'omettre ce fait, c'est de sur-revendiquer : « Mika est open source » tout
  court serait faux si la console est fermée. La formulation est donc portée sur
  **le moteur qui exécute l'agent**, jamais sur « Mika » en bloc.
- **« exportable »** : non revendiqué (F5).
- **jamais « local » sur un tenant cloud** : déjà posé et déjà gardé par
  mika#2290 (règle 5 + garde 5d). **Cité, pas re-narré** (F4), et la formulation
  est testée contre le prédicat de la garde (F6, V4).

---

## Volets d'implémentation

### V-A — La section de fait (`crates/mika-agent/src/prompt.rs`)

1. `MIKA_DOCTRINE_HEADING: &str = "## Mika Doctrine"` — en-tête anglais comme
   ses deux sœurs, corps splité par registre. L'en-tête est un marqueur de
   structure de prompt, pas du texte servi à l'utilisateur.
2. `MIKA_DOCTRINE_BODY_OPERATOR` et `MIKA_DOCTRINE_BODY_FAMILY`, chacune avec son
   doc-comment portant : les faits et leur provenance (D9), l'interdiction de
   nommer un référent spirituel (D3), et pour la famille le motif de l'absence de
   référent créateur (D4).
3. Chaque corps porte, dans l'ordre : **le nom et ses alias** (« doctrine »,
   « doctrine Mika », « tes partis pris », « en quoi tu crois », « ta
   philosophie ») ; les partis pris **avec leur pourquoi** ; les renvois
   `## Distribution Doctrine` et `## Runtime` au lieu d'une re-narration ; la
   butée topique.
4. `fn doctrine_body(persona: PersonaProfile) -> &'static str` — `match`
   exhaustif, aucun bras `_ =>`.
5. `fn write_mika_doctrine_section(prompt: &mut String, persona: PersonaProfile)`.
6. Appel dans `build_system_prompt` et `build_silent_prompt`, immédiatement après
   `write_distribution_doctrine_section`, avec le motif D8 écrit au site
   silencieux. **Pas** d'appel dans `build_compact_system_prompt`, avec le motif
   D7 écrit en commentaire au point d'omission — comme mika#2290 l'a fait
   (`prompt.rs:1645-1652`).

### V-B — La règle 6 (`write_self_identity_discipline_section`)

Ajoutée après la règle 5, avant le paragraphe de clôture auto-référentiel. Elle
porte les trois clauses de D5 : élargissement de portée, désignation de
`## Mika Doctrine` comme vérité de terrain, refus nommé de « rien trouvé qui
s'appelle ainsi ». Aucun changement de signature.

### V-C — Les tests (`prompt.rs::tests`, préfixe `mika2292_`)

Nommage et forme repris de la série `mika2290_*` déjà en place.

### V-D — Le scénario d'eval

`crates/mika-agent/tests/eval/doctrine_regressions/doctrine_mika_section_rendered.rs`,
sur le modèle de `doctrine_prompt_section_rendered.rs` (headless-safe, assertions
dures sur la forme du prompt, aucun appel réseau). Enregistré dans
`doctrine_regressions/mod.rs` avec deux entrées de vocabulaire :
`doctrine:material-doctrine-answerable` (succès post-fix) et
`doctrine:doctrine-not-found` (échec pré-fix — la réponse mesurée).

### V-E — Documentation

`crates/mika-agent/CLAUDE.md` § *Guard Fabrication Telemetry* n'est **pas**
touché (aucune garde ajoutée, D6). Une entrée dans le `CLAUDE.md` racine
(§ *MIKA_DEPLOYMENT* voisine, ou une section propre) portant : les deux registres,
le refus d'énumérer, le refus de garde et son motif mesuré, le carve-out compact,
et la sonde post-déploiement avec ses haltes.

---

## Verification contract

| id | ce qui est vérifié | comment |
|---|---|---|
| **V1** | la section est rendue dans les deux builders, dans les deux registres | 4 assertions `contains(MIKA_DOCTRINE_HEADING)` |
| **V2** | le carve-out compact tient | `build_compact_system_prompt` ne contient pas l'en-tête |
| **V3** | **aucun référent spirituel n'est nommé** — scan des deux constantes contre une liste de référents interdits | test dédié, `assert!(!body.to_lowercase().contains(r))` pour chaque `r` |
| **V4** | aucun des deux corps ne fait firer la garde 5d | `crate::evidence::guards::detect_false_local_hosting_claim(body, Deployment::Cloud).is_none()` sur les deux — le prédicat est `pub(crate)`, donc appelable depuis `prompt.rs::tests` sans élargir sa visibilité |
| **V5** | le corps famille ne porte aucun jargon d'infrastructure | modèle `mika2290_family_cloud_line_carries_no_infrastructure_jargon` |
| **V6** | le corps famille n'établit aucun référent créateur (D4) | scan de la constante |
| **V7** | « exportable » n'est revendiqué dans aucun des deux corps (F5) | scan des deux constantes |
| **V8** | `## Mika Doctrine` précède `## Self-Identity Discipline` (la règle 6 la cite) | comparaison de `find()`, modèle de l'assertion d'ordre `Runtime`/`Self-Identity` en place à `prompt.rs:5316-5320` |
| **V9** | la règle 6 est présente et nomme la section | `contains` sur l'en-tête depuis la section discipline |
| **V10** | le `match` sur `PersonaProfile` est exhaustif | structurel : le compilateur (aucun bras `_ =>` écrit) |
| **V11** | pas de régression de forme sur les sections voisines | la suite `prompt.rs::tests` existante passe inchangée |
| **V12** | `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` | CI |

**V3 est le test porteur du ticket.** Il ne vérifie pas une décision correcte, il
refuse une décision *inverse* qu'un futur éditeur bien intentionné prendra
naturellement (énumérer pour clarifier). C'est la seule assertion du lot dont
aucun test comportemental ne peut tenir lieu : l'énumération ne rendrait aucune
décision fausse, elle créerait la fuite en silence.

---

## Fire-Disposition

| ce qui peut casser | signe | disposition |
|---|---|---|
| le corps porte une phrase de localité hors des suppresseurs de 5d | V4 rouge | reformuler le corps ; **ne pas élargir `CLAIM_CONDITIONAL_MARKERS`** — son étroitesse est le motif écrit de mika#2290 |
| un `PersonaProfile` futur | erreur de compilation sur le `match` | décider explicitement pour ce registre ; c'est l'effet voulu |
| le budget de prompt | `turn_usage.system_prompt_bytes` (mika#2331) monte d'environ 1–1,5 Ko, un seul registre étant rendu | attendu ; hors chemin compact par D7 |
| la règle 6 entre en conflit avec la règle 3 | un tenant dit « je ne sais pas » sur la doctrine | c'est le cas que D5 interdit nommément ; si ça persiste, **relire d'abord si la section est bien dans le prompt servi** (sonde 1) avant de toucher au texte |

---

## Definition of Done

- [ ] `MIKA_DOCTRINE_HEADING` + les deux corps, chacun avec son doc-comment
      portant provenance des faits et motifs de refus
- [ ] `doctrine_body` / `write_mika_doctrine_section`, `match` exhaustif
- [ ] rendue dans `build_system_prompt` et `build_silent_prompt`, omise du
      compact avec le motif écrit au point d'omission
- [ ] règle 6 dans `## Self-Identity Discipline`
- [ ] V1–V11 verts, V12 vert
- [ ] scénario d'eval + vocabulaire `doctrine:*` enregistrés
- [ ] `CLAUDE.md` racine : section portant les deux registres, le refus
      d'énumérer, le refus de garde et son motif, le carve-out, la sonde
- [ ] `cargo fmt`, `cargo clippy` propres
- [ ] ticket de suivi ouvert pour F5 (« exportable »), avec sa question de
      vérification

---

## Acceptance criteria

Dérivés — le corps du ticket ne porte pas de section `## Acceptance criteria`.

- **AC1** — Une question sur « la doctrine Mika » (et ses alias « doctrine »,
  « tes partis pris », « ta philosophie », « en quoi tu crois ») ne peut plus
  recevoir « rien trouvé qui s'appelle ainsi » : la section est dans le prompt
  servi sur les deux registres et sur les deux builders non-compacts, et la
  règle 6 refuse cette réponse nommément.
- **AC2** — Le registre **Matériel** est articulé avec **son pourquoi** pour
  chaque parti pris : open source MIT (moteur), souveraineté des données,
  proactivité, mémoire persistante, croissance par invitation.
- **AC3** — Le registre **Spirituel** dispose d'une butée nette, **topique et non
  énumérative** : aucun référent (Hermétisme, sièges, Prime, le Livre) n'apparaît
  dans une constante de ce ticket. Vérifié par V3.
- **AC4** — **Faits vérifiés seulement.** MIT est revendiqué sur le moteur et
  atteste par `LICENSE`. « exportable » n'est revendiqué nulle part dans ce que
  ce ticket ajoute (V7). Aucune revendication d'hébergement local : les deux
  corps passent le prédicat de la garde 5d sous `Deployment::Cloud` (V4).
- **AC5** — **Deux registres, un fait.** La substance est servie au registre
  famille sans un seul terme d'infrastructure (V5), et le croisement
  persona × contenu est un `match` exhaustif sans bras `_ =>` (V10). Aucune
  règle dérivée de la locale ni du tenant.
- **AC6** — Le tenant d'Al — `champion`, donc outils et persona famille (F1) —
  est dans la population servie : le correctif est code-managed et atteint tout
  tenant existant au prochain déploiement, sans geste de provisionnement (F3).
- **AC7** — Le carve-out compact est préservé et **épinglé comme décision**
  (V2), rattaché à mika#1925.
- **AC8** — Aucune garde EndTurn n'est ajoutée, et ce refus est **écrit avec sa
  mesure** (D6 : le lexique spirituel est composé de mots ordinaires du registre
  famille). La moitié structurelle prise est V1 + V3.

---

## Surfaces opérateur et sonde post-déploiement

**Sonde 0 — avant toute ligne de code, une hypothèse à trancher.** La formulation
mesurée (« rien trouvé qui s'appelle… ») ressemble à un compte rendu de
**recherche**. Si le tenant a appelé `search_memory("doctrine Mika")` et a
rapporté fidèlement un résultat vide, la seconde clause de la règle 6 (D5) est le
remède exact ; sinon elle est une précaution gratuite mais inoffensive.

```sql
SELECT tool_name, input, output FROM tool_calls
 WHERE session_id = '<session du 11/09>' ORDER BY created_at;
```

À défaut de session retrouvable : armer `MIKA_LOG_LLM_BODIES` **sur mika-spirit**
(mika#2220 — armée sur le process CLI elle est inerte pour un tour servi par le
démon), redémarrer, rejouer la question, lire le bloc `## Mika Doctrine` dans le
corps de requête. **Cette sonde ne bloque pas la livraison** : la clause est
écrite dans les deux cas, la sonde décide seulement s'il faut ouvrir le suivi
d'un hard-redirect au niveau de l'outil (forme #647), qui est **hors périmètre**
ici.

**Sonde 1 — symptôme, sur un tenant cloud et sur le poste opérateur.** Rejouer
« Qu'est-ce que la doctrine Mika ? » puis « en quoi tu crois ? » puis « raconte-moi
ton origine ». Attendu : réponse substantielle sur les partis pris **avec leur
pourquoi** ; butée nette sur l'origine sans exposition et sans invention ; aucune
mention de MIT ni d'infrastructure sur le tenant champion.

**Halte 1.** Si la réponse reste « rien trouvé » : **ne pas retoucher la
formulation par réflexe**. Vérifier d'abord que la section est dans le prompt
réellement servi —

```bash
grep system_prompt_assembled "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "<tenant>") | {active_skill_count, per_skill_bytes}'
grep turn_usage "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "<tenant>") | .system_prompt_bytes'
```

`system_prompt_bytes` doit avoir monté d'environ 1–1,5 Ko par rapport à
l'avant-déploiement. S'il n'a pas bougé, le tenant est servi par le chemin compact
(D7) ou par un binaire antérieur — classe mika#2340, et c'est **le déploiement**
qu'il faut établir avant toute conclusion sur le texte.

**Halte 2 — faux positif de registre.** Si un tenant champion se met à parler de
licence, de dépôt ou d'auto-hébergement, le croisement persona × registre est
cassé : lire l'`AgentTier` résolu du tenant avant d'accuser la formulation. Un
tenant champion **provisionné avant mika-cloud#209 (2026-08-28)** porte encore
l'identité opérateur sur disque, et aucune ligne de ce ticket ne la corrige —
c'est un geste de re-provisionnement (root `CLAUDE.md` § *MIKA_AGENT_TIER*,
« What this does NOT retrofit »).

**Halte 3 — invention sur le registre spirituel.** Si un tenant fabrique du
contenu esotérique, **ne pas ajouter une garde à lexique** : D6 mesure pourquoi
elle casserait la conversation famille. Établir d'abord si la butée est dans le
prompt servi (Halte 1), puis ouvrir un ticket sur la formulation de la butée —
pas sur une détection.

**Ce que ce travail n'achète pas.** Aucun compteur, aucun événement de journal
nouveau : le défaut est une absence de réponse, et une absence ne s'émet pas. Le
seul instrument est la sonde par rejeu ci-dessus, et **le silence ne prouve rien
si personne ne pose la question** — c'est la limite que mika#2290 a déjà dû
écrire pour sa propre sonde (mika#2205).

---

## Hors périmètre (suivi à ouvrir)

1. **« exportable » revendiqué sans fonctionnalité vérifiable (F5).** Quatre
   sites : `prompt.rs:1000`, `guards.rs:2732`, `agent_loop/mod.rs:2222`,
   `docs/architecture.md:26`. Question de vérification à porter dans le ticket :
   *`mika-cloud` expose-t-il un export des données du tenant ?* Si oui, nommer la
   surface dans la doctrine ; si non, retirer le mot des quatre sites. **Ne pas
   trancher depuis ce dépôt** : la donnée du tenant n'y vit pas.
2. **Formulation « créateur » dans le registre famille (D4).** Décision de
   Vincent/Prime, une constante à changer. La butée sans référent livre en
   attendant et satisfait les deux doctrines.
3. **Hard-redirect `search_memory` sur les requêtes de forme doctrinale**
   (forme #647). **Conditionné à la sonde 0** — instruire sans mesure serait
   exactement ce que ce dépôt refuse.
4. **Variante compacte size-capped** pour MikaModel — rejoint mika#1925 avec les
   trois carve-outs existants (mika#1813, mika#1814, mika#2290).
5. **Persona champion (mika#2023).** Ce plan sert le registre famille à un
   champion, donc en français. Un champion anglophone reçoit l'image miroir du
   bug de mika#2023 : décalage de registre, pas de fuite de privilège. Écarté
   là-bas sur arbitrage de Prime (mika#2247) ; inchangé ici.
6. **Re-provisionnement des tenants champion d'avant mika-cloud#209** — geste
   opérateur, porte de lancement bloquante déjà écrite dans mika#2023, aucune
   ligne de code ici.
