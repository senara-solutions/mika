# Contributing to Mika

Mika is developed using [Claude Code](https://docs.anthropic.com/en/docs/claude-code) with the [compound engineering plugin](https://github.com/EveryInc/compound-engineering-plugin). We strongly recommend this workflow, but manual contributions are welcome too.

## Prerequisites

- **Rust** >= 1.91 (see `rust-version` in `Cargo.toml`)
- **jq** -- required by skill handler scripts
- **Claude Code** + **compound engineering plugin** (recommended, not required)

Build with `cargo build` and run tests with `cargo test`. Tests are fully mocked and do not require a `MIKA_ANTHROPIC_API_KEY`.

## Git Hooks

Enable the shared pre-commit hook to catch formatting and lint issues before they reach CI:

```bash
git config core.hooksPath .githooks
```

This runs `cargo fmt --check` and `cargo clippy` on every commit, matching the CI checks exactly.

## Garde d'écriture git hors worktree (mika#2107)

Une session Claude Code enracinée dans un **worktree lié** ne peut pas exécuter
de commande git **mutante** visant un arbre hors de ce worktree — typiquement le
checkout principal dont dépend le déploiement. Le refus arrive **avant**
l'exécution, pas après.

Elle est livrée par `.claude/settings.json`, **suivi dans le dépôt** : elle
arrive avec le checkout et n'a **aucune étape d'installation**. C'est délibéré —
les deux mécanismes de hook de ce dépôt (`.githooks` ci-dessus et
`lefthook.yml`) ont un taux d'installation mesuré de **zéro**, et mika#2107 a
établi par l'expérience que la prose ne ferme pas cette classe : un document
écrit après la troisième occurrence n'a pas empêché la quatrième, onze heures
plus tard, sur le même répertoire.

### Ce qui est refusé, et ce qui ne l'est pas

Le prédicat a quatre termes conjoints :

1. le project-dir de la session est un worktree lié (`.git` y est un fichier) ;
2. la commande est une invocation git ;
3. le verbe n'est **pas** dans l'allow-list de lecture ;
4. l'arbre cible effectif n'est pas le worktree de la session.

Le premier terme **exempte l'opérateur et l'humain par construction** : une
session enracinée dans le checkout principal ou à la racine de l'espace de
travail n'est jamais dans la population, donc `/mika-platform-sync-main` et la
maintenance ordinaire ne paient rien.

Le troisième est une **allow-list de lecture, pas une deny-list de mutation**.
Une deny-list bâtie sur les mécanismes connus (`checkout`, `reset`) aurait
manqué la moitié des occurrences mesurées — dont `git add`, la seule qui ait
réellement expédié du code non revu en production. Tout verbe non classé est
donc refusé lorsqu'il vise hors du worktree.

### Lever un refus

Le message de refus nomme l'arbre visé, le worktree de la session, le remède et
la dérogation. Dans l'ordre de préférence :

```bash
git -C "$CLAUDE_PROJECT_DIR" <commande>     # viser son propre worktree
MIKA_GUARD_SHARED_CHECKOUT=0 <commande>     # dérogation explicite, journalisée
```

### Sonde d'armement — et pourquoi elle est porteuse

La garde est **fail-open** : un script absent, illisible ou rendant une sortie
invalide **laisse passer** la commande. Un fail-closed coucherait toutes les
sessions du dépôt — dispatches et orchestrateur en incident compris — pour un
défaut de garde qui ne protège qu'une population résiduelle.

**Ce que cet arbitrage coûte, écrit plutôt que découvert : une garde cassée se
lit exactement comme une garde qui n'a jamais eu à firer.** D'où la ligne
d'armement, qui est la seule chose distinguant les deux :

```bash
grep shared-checkout-guard ~/.mika/state/shared-checkout-guard.log
```

- `shared-checkout-guard: armed` à chaque démarrage de session → la garde est
  chargée. **Son absence est l'information** : elle dit que le hook n'est pas
  chargé, et non que rien n'a eu à être refusé.
- `deny project=… target=… verb=…` → un refus. **Régime attendu : faible mais
  non nul.** Un flux soutenu **ne se traite pas en élargissant la garde** : il
  dit que les consignes de spawn font dériver le répertoire courant, et c'est
  *cela* qu'il faut traiter.
- `bypass MIKA_GUARD_SHARED_CHECKOUT=0` → une dérogation. Un contournement
  silencieux serait pire que pas de garde ; celui-ci est daté.

Le chemin du journal est surchargeable par `MIKA_GUARD_SHARED_CHECKOUT_LOG`.
**L'échec d'écriture du journal ne change jamais la décision** — journaliser est
une observation, pas un terme du prédicat ; une garde qui refuserait parce
qu'elle n'a pas pu écrire son log serait un fail-closed déguisé.

### Portée

La garde ne gouverne que les sessions enracinées dans **ce** dépôt : elle ferme
deux des quatre occurrences mesurées. Les deux autres visaient le checkout
`claude-pilot`, monté en installation éditable — celui où « changer un fichier »
et « déployer » sont le même acte. `scripts/guard-shared-checkout` est
volontairement agnostique du dépôt pour que son portage y soit un vendoring plus
trois lignes de `settings.json`, pas une réécriture.

Elle défend contre l'**accident**, pas contre un adversaire : le bac à sable des
dispatches (mika#2141) est le mur, cette garde est le garde-fou.

Tests : `scripts/test-guard-shared-checkout.sh` (comportement, les quatre
occurrences en fixtures) et `scripts/check-shared-checkout-guard-wiring.sh`
(câblage), tous deux exécutés par le job CI `shared-checkout-guard-lint`.

## Development Workflow with Claude Code

The recommended workflow uses the `/mika` slash command, which chains every step from planning through documentation:

```
/mika <description of what you want to build or fix>
```

This runs the following steps in order:

1. **Plan** (`/ce:plan`) -- Research the codebase, design the approach, write a plan file to `docs/plans/`
2. **Work** (`/ce:work`) -- Implement the plan with incremental commits and continuous testing
3. **Review** (`/ce:review`) -- Multi-agent code review for quality and correctness
4. **Resolve TODOs** (`/compound-engineering:resolve_todo_parallel`) -- Address review findings tracked in `todos/`
5. **Doc Audit** (`/mika-doc-audit`) -- Update documentation based on code changes
6. **Compound** (`/ce:compound`) -- Document the solution in `docs/solutions/` for institutional knowledge

The command uses a `/ralph-loop` wrapper internally to ensure all steps run to completion without stopping between them.

For documentation-only changes, you can run `/mika-doc-audit` directly instead of the full `/mika` workflow.

### Issue Management

Two slash commands streamline GitHub issue creation using the repo's label taxonomy:

```
/mika-issue <description>       # Create a single issue
/mika-issues <list of issues>   # Batch-create multiple issues
```

Both commands classify issues by type (`bug`, `enhancement`, `documentation`, `question`), assign priority (`p0-critical` through `p3-nice-to-have`) and component labels (`agent-core`, `tui`, `team-engine`, `skill`, `gateway`, `infrastructure`), format the body using the repo's issue templates, and present the `gh issue create` command for approval before executing.

### Setup

1. Install [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
2. Install the [compound engineering plugin](https://github.com/EveryInc/compound-engineering-plugin)
3. Run `/mika` from the project root

## Manual Workflow

If you prefer not to use Claude Code:

```bash
# 1. Create a feature branch (type/description-kebab-case)
git checkout -b feat/my-feature

# 2. Make changes and run quality gates (these match CI exactly)
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test

# 3. Commit with conventional format (see below)
git commit -m "feat(agent): add new capability"

# 4. Push and open a PR
git push -u origin feat/my-feature
```

Branch types: `feat/`, `fix/`, `refactor/`, `docs/`, `chore/`.

## Commit Conventions

We use [Conventional Commits](https://www.conventionalcommits.org/). Automated changelog generation is currently **disabled** (see `docs/deployment.md` and mika#2048); the convention still governs commit messages here. Use these prefixes:

**Appears in changelog:**

| Prefix | Changelog Group | Example |
|--------|----------------|---------|
| `feat` | Added | `feat(tui): add model switching` |
| `fix` | Fixed | `fix(agent): handle empty response` |
| `refactor` | Changed | `refactor: extract config module` |
| `perf` | Performance | `perf(search): optimize FTS5 queries` |
| `doc`/`docs` | Documentation | `doc: update architecture guide` |

**Skipped in changelog** (still valid):

| Prefix | Use for |
|--------|---------|
| `test` | Test additions or fixes |
| `ci` | CI/CD pipeline changes |
| `chore` | Dependency updates, tooling |
| `style` | Formatting (cargo fmt) |

Scopes are optional. Common scopes: `agent`, `tui`, `gateway`, `common`, `cli`.

## Testing

- Tests live inline in each module: `#[cfg(test)] mod tests`
- No API key is required -- tests are fully mocked
- Some tests use `serial_test` for isolation; respect `#[serial]` annotations
- Add tests for new functionality, covering validation, success paths, and edge cases

## Documentation

When your changes affect behavior, update the relevant docs. The `/mika-doc-audit` step handles this automatically when using the Claude Code workflow.

| What Changed | Update |
|-------------|--------|
| Significant feature or behavior changes | `CLAUDE.md` (project instructions) |
| Environment variables | `.env.example` |
| Architecture or agent loop | `docs/architecture.md` |
| Configuration or settings | `docs/configuration.md` |
| Skills system | `docs/skills.md` |
| TUI slash commands | `docs/slash-commands.md` |
| Deployment or Docker | `docs/deployment.md` |
| New user-facing features | `docs/getting-started.md` |
| Public API changes | `README.md` |

For significant architectural changes, add an ADR to `docs/adr/` following the existing sequential numbering and Context/Decision/Consequences format.

## Security Guidelines

- Never log API keys or secrets
- `Settings` has a manual `Debug` impl that redacts sensitive fields -- follow this pattern
- Child processes use `env_clear()` + allowlist to prevent `MIKA_*` env var leakage
- Handler scripts `unset` MIKA env vars before executing commands
- When adding tools that spawn processes, use the existing `env_clear()` pattern in `McpManager`

## License

By contributing to Mika, you agree that your contributions will be licensed under the [MIT License](LICENSE).
