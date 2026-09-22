use anyhow::{Context, Result, anyhow, bail};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use mika_common::agent::{self, DEFAULT_AGENT};
use mika_common::home::{self, AgentTier};

use crate::cli::{AgentsArgs, AgentsCommand, OutputFormat};
use crate::wizard;
use mika_agent::db::{Database, ResetAgentCounts};

pub async fn run(args: AgentsArgs) -> Result<()> {
    let global_home = home::resolve_home_dir()?;

    // Auto-migrate on any agents subcommand
    home::migrate_to_multi_agent(&global_home)?;

    match args.command {
        AgentsCommand::List { format } => list(&global_home, &format),
        AgentsCommand::Create {
            name,
            no_interactive,
        } => create(&global_home, &name, no_interactive).await,
        AgentsCommand::Delete { name, force } => delete(&global_home, &name, force),
        AgentsCommand::Switch { name } => switch(&global_home, &name),
        AgentsCommand::Clone { source, name } => clone(&global_home, &source, &name),
        AgentsCommand::Validate { name, format } => {
            validate_agents(&global_home, name.as_deref(), &format)
        }
        AgentsCommand::Reset {
            name,
            force,
            dry_run,
            yes,
        } => reset(&global_home, &name, force, dry_run, yes),
        AgentsCommand::Reprovision {
            name,
            tier,
            identity_only,
            dry_run,
            yes,
        } => reprovision(
            &global_home,
            &name,
            ReprovisionOptions {
                tier: tier.as_deref(),
                identity_only,
                dry_run,
                yes,
            },
            &mut io::stdout(),
        ),
        AgentsCommand::Budget { agent, format } => {
            let name = agent
                .or(args.agent_flag.agent)
                .unwrap_or_else(|| home::read_active_agent(&global_home));
            budget(&name, &format, &mut io::stdout()).await
        }
    }
}

/// `mika agents budget` — render the budget record **mika-spirit attested**.
///
/// # Why nothing is resolved here
///
/// `BudgetProvenance::resolve` reads the `process_env` **of the process that
/// calls it**. A subcommand resolving locally would report `agent_config` at 240
/// while mika-spirit runs at 300 under a service variable — a field asserting,
/// with authority, a setting that is not in force. That is mika#2304's defect
/// verbatim (`--verbose` printing the requested model while the turn ran under
/// another) and mika#2270's lesson (the server held the answer and discarded
/// it). So: **spirit attests, the CLI renders. Never the reverse.**
///
/// The structural half of that rule is
/// `budget_tests::mika2457_the_cli_resolves_no_budget_locally`, a source scan
/// with an allowlist shipped empty. When it fires, the second reader is removed
/// — not allowlisted.
///
/// # "Not attested" is an answer, and it is the honest one
///
/// Spirit unreachable, or answering 404, prints *"not attested"* and **no
/// values**. That population is ambiguous — a binary predating this change, an
/// agent this server does not serve, a daemon that is down — and that ambiguity
/// is preferable to a wrong number, which is the population mika#2304 had to
/// name for exactly this reason.
///
/// # The cause is said, even though the values are not
///
/// "No values" does not license a false *explanation*. A `401` (this shell's
/// `MIKA_INTERNAL_TOKEN` is not the server's), a `5xx`, or a body the mirror
/// cannot read (version skew) are not "unreachable, predating the fix, or not
/// serving this agent", and printing that sentence for them sends the operator
/// after the wrong remedy. So the fetch returns an [`UnattestedCause`] and the
/// render names it — still with no budget value.
async fn budget(name: &str, format: &OutputFormat, out: &mut impl Write) -> Result<()> {
    let base = crate::commands::dashboard::spirit_url();
    let url = format!(
        "{}/api/v1/agents/{}/budget",
        base.trim_end_matches('/'),
        name
    );
    let token = crate::commands::dashboard::auth_token()?;

    let response = reqwest::Client::new()
        .get(&url)
        .header("authorization", format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let record = match response {
        Err(_) => Err(UnattestedCause::Unreachable),
        Ok(resp) if !resp.status().is_success() => {
            Err(UnattestedCause::Status(resp.status().as_u16()))
        }
        Ok(resp) => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| {
                let mut record =
                    serde_json::from_value::<BudgetRecord>(v["budget"].clone()).ok()?;
                // The drift is a **sibling** of the record, not a field of it
                // (KTD3), and it is read tolerantly: absent (a spirit predating
                // mika#2473) or unreadable (a shape this mirror does not know)
                // both give `None`, which renders as its own sentence rather
                // than failing the whole attestation over a second opinion.
                record.model_drift =
                    serde_json::from_value::<ModelDrift>(v["model_drift"].clone()).ok();
                Some(record)
            })
            .ok_or(UnattestedCause::Malformed),
    };

    let record = match record {
        Ok(record) => record,
        Err(cause) => return render_unattested(name, format, &base, cause, out),
    };

    match format {
        OutputFormat::Json => writeln!(out, "{}", serde_json::to_string_pretty(&record)?)?,
        OutputFormat::Yaml => writeln!(out, "{}", serde_yaml::to_string(&record)?)?,
        OutputFormat::Text => render_budget_text(&record, out)?,
    }
    Ok(())
}

/// The record as mika-spirit serves it.
///
/// Deserialized into a local mirror rather than importing
/// `mika_common::llm::ResolvedBudgetRecord`: the CLI may be talking to a spirit
/// of another version, so an unknown field must not fail the render. The fields
/// below are the ones this surface prints.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct BudgetRecord {
    agent_id: String,
    http_timeout_secs: u64,
    agent_total_timeout_secs: u64,
    max_attempts: u32,
    effective_max_attempts: u32,
    retry_reachable: bool,
    llm_max_tokens: u32,
    reachable_output_tokens: u64,
    http_source: String,
    total_source: String,
    max_tokens_source: String,
    provider: String,
    provider_source: String,
    model: String,
    model_source: String,
    model_config_key: String,
    resolved_at: String,
    /// The mika#2473 sibling, carried on the record so one value feeds all
    /// three renders. `None` is *"this server did not evaluate it"* — never
    /// *"in phase"*; see [`render_drift_line`].
    #[serde(default)]
    model_drift: Option<ModelDrift>,
}

/// The code↔runtime drift as mika-spirit establishes it at `init_agent`
/// (mika#2473).
///
/// A local mirror of `mika_common::llm::ModelDriftCheck` rather than an import,
/// for the reason [`BudgetRecord`] gives above: this CLI may be talking to a
/// spirit of another version, so neither an unknown field nor an unknown
/// `status` word may fail the render. Hence `status: String` rather than an
/// enum, and every arm-specific field an `Option` — the wire form is
/// internally tagged, and each arm carries only what it honestly knows.
///
/// `runtime_provider_source` and `model_config_key` exist on the wire's `drift`
/// arm and are deliberately not mirrored: this surface prints neither, and a
/// mirror field nobody reads is a second place for the two shapes to diverge.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ModelDrift {
    /// `in_sync` / `drift` / `not_applicable` — or a word a newer spirit knows
    /// and this binary does not.
    status: String,
    /// The provider `well_known_agents.rs` selects. Absent on `not_applicable`.
    declared_provider: Option<String>,
    /// The model `well_known_agents.rs` declares. Absent on `not_applicable`.
    declared_model: Option<String>,
    /// The provider in force. Present on `drift` only.
    runtime_provider: Option<String>,
    /// The model actually served. Present on `drift` only.
    runtime_model: Option<String>,
    /// The door that model came through. Present on `drift` only — and
    /// **absent on `in_sync` by design**: on that arm the runtime values equal
    /// the declared ones and the emitter has no honest source for a door, so
    /// the enum does not carry one.
    runtime_model_source: Option<String>,
}

/// What a `drift` field a newer spirit did not send is rendered as.
///
/// Version skew inside an arm: printing an empty string there would read as
/// "no model", which is not what is known.
const DRIFT_FIELD_ABSENT: &str = "(non transmis)";

/// mika#2328's sixth provenance word: a door carried an `llm_provider` the
/// reader cannot parse, so the model key's *name* is unknown.
///
/// Pre-existing behaviour (`budget_provenance.rs`), not introduced here — this
/// surface only has to not lose it in transit. Reporting `default` there would
/// state "no door carried the model", which is unknown and possibly false.
const MODEL_SOURCE_UNKNOWN_PROVIDER: &str = "unknown_provider";

fn render_budget_text(record: &BudgetRecord, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "{:<32} (résolu le {})",
        record.agent_id, record.resolved_at
    )?;
    writeln!(
        out,
        "  provider   {:<20} ({})",
        record.provider, record.provider_source
    )?;
    if record.model_source == MODEL_SOURCE_UNKNOWN_PROVIDER {
        // `effective_model()` is `None` on this crossing, so the field arrives
        // empty. Printing an empty value would read as "no model", which is not
        // what is known — what is known is that the provider is unreadable.
        writeln!(
            out,
            "  model      (non résolu — provider illisible : \"{}\")",
            record.provider
        )?;
    } else {
        writeln!(
            out,
            "  model      {:<20} ({}, clé: {})",
            record.model, record.model_source, record.model_config_key
        )?;
    }
    render_drift_line(record.model_drift.as_ref(), out)?;
    writeln!(
        out,
        "  plafond    {:<20} ({})",
        format!("{} s", record.http_timeout_secs),
        record.http_source
    )?;
    writeln!(
        out,
        "  enveloppe  {:<20} ({})",
        format!("{} s", record.agent_total_timeout_secs),
        record.total_source
    )?;
    writeln!(
        out,
        "  max_tokens {:<20} ({})",
        record.llm_max_tokens, record.max_tokens_source
    )?;
    writeln!(
        out,
        "  atteignable {} tokens",
        record.reachable_output_tokens
    )?;
    writeln!(
        out,
        "  tentatives {} nominales / {} atteignables{}",
        record.max_attempts,
        record.effective_max_attempts,
        if record.retry_reachable {
            ""
        } else {
            "  ⚠ dernière tentative nominale inatteignable (mika#2362)"
        }
    )?;
    Ok(())
}

/// The mika#2473 `code` line: what the repo declares, and whether the runtime
/// serves it — read directly under the `model` line it qualifies.
///
/// # The absent key is a state, not an `in_sync`
///
/// A spirit predating mika#2473 answers `{budget}` with no `model_drift`. A
/// render that folded that silence into « en phase » would attest a
/// comparison nobody ran — mika#2457's *not attested* population, one field
/// over — so it gets its own sentence, and so does a `status` word this binary
/// does not know.
///
/// # `in_sync` promises no door
///
/// The `InSync` arm of `ModelDriftCheck` carries only the declared pair: there
/// the runtime values equal the declared ones by definition and the emitter has
/// no honest source for the `*_source` fields. This line therefore names no
/// provenance on that arm — inventing one would be the false provenance
/// `budget_provenance.rs` refuses in so many words.
fn render_drift_line(drift: Option<&ModelDrift>, out: &mut impl Write) -> Result<()> {
    let Some(drift) = drift else {
        return Ok(writeln!(
            out,
            "  code       (dérive non évaluée par ce serveur — binaire antérieur à mika#2473)"
        )?);
    };
    let declared = || {
        drift
            .declared_model
            .as_deref()
            .unwrap_or(DRIFT_FIELD_ABSENT)
            .to_string()
    };
    match drift.status.as_str() {
        "not_applicable" => writeln!(
            out,
            "  code       (aucun modèle déclaré par le code pour cet agent)"
        )?,
        "in_sync" => writeln!(
            out,
            "  code       {:<20} (well_known_agents.rs) — en phase",
            declared()
        )?,
        "drift" => writeln!(
            out,
            "  code       {:<20} (well_known_agents.rs) — DÉRIVE : le runtime sert {} ({})",
            declared(),
            drift.runtime_model.as_deref().unwrap_or(DRIFT_FIELD_ABSENT),
            drift
                .runtime_model_source
                .as_deref()
                .unwrap_or(DRIFT_FIELD_ABSENT)
        )?,
        other => writeln!(
            out,
            "  code       (état de dérive « {other} » inconnu de ce CLI — spirit plus récent que ce binaire)"
        )?,
    }
    Ok(())
}

/// Why the server attested nothing (mika#2457).
///
/// Every variant still renders **no value**; what they separate is the remedy.
/// A `404` keeps the ambiguous sentence (a binary predating the route and an
/// agent this server does not serve answer identically), the others do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnattestedCause {
    /// No HTTP answer at all: refused, timed out, DNS — the daemon is down or
    /// `MIKA_SPIRIT_URL` points elsewhere.
    Unreachable,
    /// The server answered with a non-2xx status.
    Status(u16),
    /// A 2xx whose body this CLI's mirror cannot read — version skew between the
    /// CLI and the spirit it talks to.
    Malformed,
}

impl UnattestedCause {
    /// Wire value of the `reason` field in JSON/YAML — scripts branch on it.
    fn reason(self) -> &'static str {
        match self {
            UnattestedCause::Unreachable => "unreachable",
            UnattestedCause::Status(401 | 403) => "unauthorized",
            UnattestedCause::Status(404) => "not_found",
            UnattestedCause::Status(_) => "http_error",
            UnattestedCause::Malformed => "malformed_response",
        }
    }

    /// The one sentence the text render gives as the cause.
    fn explain(self, base: &str) -> String {
        match self {
            UnattestedCause::Unreachable => {
                format!("  mika-spirit à {base} est injoignable.")
            }
            UnattestedCause::Status(code @ (401 | 403)) => format!(
                "  mika-spirit à {base} a refusé le jeton ({code}) : le \
                 MIKA_INTERNAL_TOKEN de ce shell n'est pas celui du serveur."
            ),
            UnattestedCause::Status(404) => format!(
                "  mika-spirit à {base} a répondu 404 : binaire antérieur au \
                 correctif, ou il ne sert pas cet agent."
            ),
            UnattestedCause::Status(code) => {
                format!("  mika-spirit à {base} a répondu {code}.")
            }
            UnattestedCause::Malformed => format!(
                "  mika-spirit à {base} a répondu, mais son record est illisible \
                 par ce CLI (versions divergentes ?)."
            ),
        }
    }
}

/// Render the "this server attested nothing" population — with no values.
fn render_unattested(
    name: &str,
    format: &OutputFormat,
    base: &str,
    cause: UnattestedCause,
    out: &mut impl Write,
) -> Result<()> {
    // One value for both structured formats, so a field added here cannot
    // reach one serializer and miss the other.
    let mut payload = serde_json::json!({
        "agent_id": name,
        "attested": false,
        "spirit_url": base,
        "reason": cause.reason(),
    });
    if let UnattestedCause::Status(code) = cause {
        payload["status"] = serde_json::json!(code);
    }
    match format {
        OutputFormat::Json => writeln!(out, "{}", serde_json::to_string_pretty(&payload)?)?,
        OutputFormat::Yaml => writeln!(out, "{}", serde_yaml::to_string(&payload)?)?,
        OutputFormat::Text => {
            writeln!(out, "{name}")?;
            writeln!(out, "  (non attesté — ce serveur n'a rien attesté)")?;
            writeln!(out, "{}", cause.explain(base))?;
            writeln!(
                out,
                "  Aucune valeur locale n'est affichée : la résoudre ici lirait \
                 l'environnement de CE process, pas celui du serveur."
            )?;
        }
    }
    Ok(())
}

fn list(global_home: &std::path::Path, format: &OutputFormat) -> Result<()> {
    let agents = agent::list_agents(global_home);
    let active = home::read_active_agent(global_home);

    match format {
        OutputFormat::Json => {
            let entries: Vec<serde_json::Value> = agents
                .iter()
                .map(|name| {
                    serde_json::json!({
                        "name": name,
                        "active": *name == active,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
        OutputFormat::Yaml => {
            let entries: Vec<serde_json::Value> = agents
                .iter()
                .map(|name| {
                    serde_json::json!({
                        "name": name,
                        "active": *name == active,
                    })
                })
                .collect();
            print!("{}", serde_yaml::to_string(&entries)?);
        }
        OutputFormat::Text => {
            if agents.is_empty() {
                println!("\n  No agents found. Run `mika agents create <name>` to create one.\n");
                return Ok(());
            }

            println!("\n  Agents:");
            for name in &agents {
                let marker = if *name == active { " (active)" } else { "" };
                println!("    {name}{marker}");
            }
            println!();
        }
    }
    Ok(())
}

async fn create(global_home: &std::path::Path, name: &str, no_interactive: bool) -> Result<()> {
    let name = agent::normalize_agent_name(name);
    agent::validate_agent_name(&name)?;

    if agent::agent_exists(global_home, &name) {
        bail!("Agent '{name}' already exists.");
    }

    // Ensure agents/ dir exists (for fresh installs that aren't legacy)
    std::fs::create_dir_all(global_home.join("agents"))?;

    home::bootstrap_agent(global_home, &name)?;

    let interactive = !no_interactive && std::io::stdin().is_terminal();

    if interactive {
        let result = wizard::run_agent_wizard(&name)?;
        let agent_home = mika_common::agent::agent_dir(global_home, &name);

        // Overwrite identity.toml with wizard answers
        let identity = format!(
            "name = \"{}\"\nemoji = \"{}\"\n",
            result.display_name, result.emoji
        );
        std::fs::write(agent_home.join("identity.toml"), identity)?;

        // Generate or template soul.md if specialization was provided
        if !result.specialization.is_empty() {
            let soul = match try_generate_soul(global_home, &name, &result).await {
                Some(generated) => generated,
                None => wizard::template_soul_md(
                    &result.display_name,
                    &result.specialization,
                    &result.communication_style,
                ),
            };
            std::fs::write(agent_home.join("soul.md"), soul)?;
        }
    }

    // Always seed on explicit creation, regardless of disable_bundled_skills config
    let agent_home = mika_common::agent::agent_dir(global_home, &name);
    mika_agent::startup::seed_bundled_skills_if_needed(&agent_home, false);

    println!("\n  Created agent '{name}'.");
    println!("  Use `mika --agent {name}` or `mika agents switch {name}` to use it.\n");
    Ok(())
}

/// Try to generate soul.md via LLM. Returns None on any failure.
async fn try_generate_soul(
    global_home: &std::path::Path,
    name: &str,
    result: &wizard::AgentWizardResult,
) -> Option<String> {
    let settings = mika_common::config::Settings::load(global_home).ok()?;
    let provider = settings.make_llm_provider().ok()?;

    println!("  Generating personality...");
    match wizard::generate_soul_md(
        provider.as_ref(),
        name,
        &result.specialization,
        &result.communication_style,
    )
    .await
    {
        Some(soul) => Some(soul),
        None => {
            println!("  Could not generate personality, using template.");
            None
        }
    }
}

fn delete(global_home: &std::path::Path, name: &str, force: bool) -> Result<()> {
    let name = agent::normalize_agent_name(name);

    if name == DEFAULT_AGENT {
        bail!("Cannot delete the default agent '{DEFAULT_AGENT}'.");
    }

    if !agent::agent_exists(global_home, &name) {
        bail!("Agent '{name}' not found.");
    }

    if !force {
        print!("  Delete agent '{name}' and all its data? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    let dir = agent::agent_dir(global_home, &name);
    std::fs::remove_dir_all(&dir)?;

    // If deleted agent was active, switch back to main
    let active = home::read_active_agent(global_home);
    if active == name {
        home::write_active_agent(global_home, DEFAULT_AGENT)?;
        println!("  Switched active agent to '{DEFAULT_AGENT}'.");
    }

    println!("  Deleted agent '{name}'.");
    Ok(())
}

fn switch(global_home: &std::path::Path, name: &str) -> Result<()> {
    let name = agent::normalize_agent_name(name);
    agent::validate_agent_name(&name)?;

    if !agent::agent_exists(global_home, &name) {
        bail!("Agent '{name}' not found. Create it with `mika agents create {name}`.");
    }

    home::write_active_agent(global_home, &name)?;
    println!("\n  Switched to agent '{name}'.\n");
    Ok(())
}

fn clone(global_home: &std::path::Path, source: &str, target: &str) -> Result<()> {
    let source = agent::normalize_agent_name(source);
    let target = agent::normalize_agent_name(target);
    agent::validate_agent_name(&target)?;

    if !agent::agent_exists(global_home, &source) {
        bail!("Source agent '{source}' not found.");
    }
    if agent::agent_exists(global_home, &target) {
        bail!("Agent '{target}' already exists.");
    }

    // Bootstrap the new agent with defaults first
    home::bootstrap_agent(global_home, &target)?;

    let src_dir = agent::agent_dir(global_home, &source);
    let dst_dir = agent::agent_dir(global_home, &target);

    // Copy personality files (overwrite the defaults)
    for filename in &[
        "soul.md",
        "identity.toml",
        "config.toml",
        "heartbeat.md",
        "user.md",
    ] {
        let src = src_dir.join(filename);
        if src.is_file() {
            std::fs::copy(&src, dst_dir.join(filename))?;
        }
    }

    // Copy skills directory
    let src_skills = src_dir.join("skills");
    if src_skills.is_dir() {
        copy_dir_recursive(&src_skills, &dst_dir.join("skills"), 0)?;
    }

    println!("\n  Cloned '{source}' personality into new agent '{target}'.");
    println!("  The new agent starts with a fresh database (no conversation history).\n");
    Ok(())
}

fn validate_agents(
    global_home: &std::path::Path,
    name: Option<&str>,
    format: &OutputFormat,
) -> Result<()> {
    use mika_agent::skills::index::DiagnosticLevel;
    use mika_agent::validate::validate_agent;

    let agents: Vec<String> = match name {
        Some(n) => {
            agent::validate_agent_name(n)?;
            if !agent::agent_exists(global_home, n) {
                match format {
                    OutputFormat::Json => println!("[]"),
                    OutputFormat::Yaml => print!(
                        "{}",
                        serde_yaml::to_string(&Vec::<serde_json::Value>::new())?
                    ),
                    OutputFormat::Text => println!("\n  Agent '{n}' not found.\n"),
                }
                return Ok(());
            }
            vec![n.to_string()]
        }
        None => {
            let found = agent::list_agents(global_home);
            if found.is_empty() {
                match format {
                    OutputFormat::Json => println!("[]"),
                    OutputFormat::Yaml => print!(
                        "{}",
                        serde_yaml::to_string(&Vec::<serde_json::Value>::new())?
                    ),
                    OutputFormat::Text => println!("\n  No agents found.\n"),
                }
                return Ok(());
            }
            found
        }
    };

    let mut all_diags: Vec<serde_json::Value> = Vec::new();
    let mut total_errors = 0;
    let mut total_warnings = 0;

    if matches!(format, OutputFormat::Text) {
        println!();
    }

    for agent_name in &agents {
        let diags = validate_agent(global_home, agent_name);
        let has_errors = diags.iter().any(|d| d.level == DiagnosticLevel::Fail);
        let has_warnings = diags.iter().any(|d| d.level == DiagnosticLevel::Warn);

        if has_errors {
            total_errors += 1;
        }
        if has_warnings {
            total_warnings += 1;
        }

        match format {
            OutputFormat::Json | OutputFormat::Yaml => {
                for diag in &diags {
                    all_diags.push(serde_json::json!({
                        "agent": agent_name,
                        "level": diag.level,
                        "message": diag.message,
                    }));
                }
            }
            OutputFormat::Text => {
                println!("  {agent_name}/");
                for diag in &diags {
                    println!("    {} {}", diag.tag(), diag.message);
                }
            }
        }
    }

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&all_diags)?);
        }
        OutputFormat::Yaml => {
            print!("{}", serde_yaml::to_string(&all_diags)?);
        }
        OutputFormat::Text => {
            println!();
            let total = agents.len();
            let ok_count = total - total_errors;
            if total_errors == 0 && total_warnings == 0 {
                println!("  All {total} agent(s) valid.");
            } else {
                println!(
                    "  {ok_count}/{total} valid, {total_errors} with errors, {total_warnings} with warnings."
                );
            }
            println!();
        }
    }

    if total_errors > 0 {
        bail!("agent validation failed: {} error(s) found", total_errors);
    }

    Ok(())
}

fn reset(
    global_home: &std::path::Path,
    name: &str,
    force: bool,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    let name = agent::normalize_agent_name(name);

    if !agent::agent_exists(global_home, &name) {
        bail!("Agent '{name}' not found.");
    }

    // Open the database
    let db_path = home::container_db_path(global_home);
    if !db_path.exists() {
        bail!(
            "Mika database not found at {}. Run 'mika status' to initialize.",
            db_path.display()
        );
    }
    let db = Database::open(&db_path)?;

    // Ensure the agent is registered in DB
    let agents_db = db.list_agents_db()?;
    let agent_id = match agents_db.iter().find(|a| a.id == name || a.name == name) {
        Some(a) => a.id.clone(),
        None => bail!("Agent '{name}' exists on disk but not in the database."),
    };

    // Active-task guard
    let active_tasks = db.get_active_tasks_for_agent(&agent_id)?;
    if !active_tasks.is_empty() && !force {
        println!("\n  Cannot reset agent '{name}' — active tasks found:\n");
        for (task_id, status) in &active_tasks {
            println!("    {task_id}  ({status})");
        }
        println!("\n  Use --force to bypass this check, or cancel active tasks first.\n");
        bail!("agent_busy: {} active task(s)", active_tasks.len());
    }

    // Dry-run: just show counts
    if dry_run {
        let counts = db.count_agent_state(&agent_id)?;
        println!("\n  Dry run — rows that would be deleted for agent '{name}':\n");
        print_counts(&counts);
        println!("    {:<40} {}", "Total", counts.total());
        println!();
        return Ok(());
    }

    // Confirmation prompt
    if !yes {
        if !io::stdin().is_terminal() {
            bail!(
                "Non-interactive terminal requires --yes flag to bypass confirmation. \
                 Use: mika agents reset {name} --yes"
            );
        }

        // Show preview before asking
        let counts = db.count_agent_state(&agent_id)?;
        println!(
            "\n  This will delete {} row(s) across all tables for agent '{name}'.",
            counts.total()
        );
        println!("  The agent row and identity.toml will be preserved.\n");

        print!("  Type the agent name to confirm: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();
        if input != name {
            println!("  Name mismatch — aborting.");
            return Ok(());
        }
    }

    // Execute reset
    let counts = db.reset_agent_state(&agent_id)?;

    println!("\n  Reset complete for agent '{name}':\n");
    print_counts(&counts);
    println!("    {:<40} {}", "Total", counts.total());
    println!();
    println!("  Custom skill overrides (LLM model routing) have been cleared.");
    println!("  Re-apply via `mika skills llm set` if needed.");
    println!("  Bundled skills will be restored on next startup.\n");

    Ok(())
}

fn print_counts(counts: &ResetAgentCounts) {
    let rows: &[(&str, u64)] = &[
        ("sessions", counts.sessions),
        ("messages", counts.messages),
        ("core_memory", counts.core_memory),
        ("llm_calls", counts.llm_calls),
        ("tool_calls", counts.tool_calls),
        ("audit_events", counts.audit_events),
        ("audit_event_summaries", counts.audit_event_summaries),
        ("people", counts.people),
        ("commitments", counts.commitments),
        ("preferences", counts.preferences),
        ("events", counts.events),
        ("search_content", counts.search_content),
        ("tasks", counts.tasks),
        ("kg_subject_resolutions", counts.kg_subject_resolutions),
        ("kg_resolutions_log", counts.kg_resolutions_log),
        ("agent_kg_corpora", counts.agent_kg_corpora),
        ("kg_invalidated_no_match", counts.kg_invalidated_no_match),
        ("skill_overrides", counts.skill_overrides),
        ("heartbeat_sends", counts.heartbeat_sends),
        ("reflection_runs", counts.reflection_runs),
        ("customer_config", counts.customer_config),
        ("failed_sends", counts.failed_sends),
        ("kg_chunks", counts.kg_chunks),
        ("kg_subject_entities", counts.kg_subject_entities),
        ("kg_subject_relationships", counts.kg_subject_relationships),
        ("kg_chunk_subjects", counts.kg_chunk_subjects),
        (
            "kg_chunk_subject_relationships",
            counts.kg_chunk_subject_relationships,
        ),
        ("kg_extractions", counts.kg_extractions),
    ];
    for (name, count) in rows {
        if *count > 0 {
            println!("    {:<40} {}", name, count);
        }
    }
}

// ---------------------------------------------------------------------------
// `mika agents reprovision` (mika#2230)
// ---------------------------------------------------------------------------
//
// Re-applies the authoritative `identity.toml` **and** `soul.md` for an agent
// that already exists on disk. Until this verb, that gesture had no tooled path
// at all: `bootstrap_fresh_install` never runs twice (`home::is_initialized` is
// true for any tenant whose `data/mika.db` exists), `write_default_if_missing`
// never overwrites, and `mika agents create` refuses an existing agent — so the
// only route was hand-copying constants out of the source tree.
//
// Two halves, deliberately, because a tier has two axes since mika#2023: writing
// `identity.toml` alone fabricates exactly the drift the mika#1962 boot guard
// exists to catch (a family allowlist under an operator persona, or the reverse).
// `--identity-only` keeps that escape hatch and *says* what it costs.

/// The flags of one `reprovision` invocation, gathered so the signature stays
/// readable and clippy's `too_many_arguments` stays quiet.
struct ReprovisionOptions<'a> {
    tier: Option<&'a str>,
    identity_only: bool,
    dry_run: bool,
    yes: bool,
}

/// A tier, where it came from, and whether the operator's word was recognized.
struct ResolvedTier {
    tier: AgentTier,
    /// Human-readable provenance, printed in the pre-digest. `llm_budget_resolved`
    /// (mika#2293) is the model: *a setting you cannot observe is not a setting.*
    provenance: String,
    /// `Some(raw)` when the value was non-empty and outside the tier vocabulary.
    /// The tier still resolves — fail-closed, mika#2023 AC2 — and the pre-digest
    /// names the offending value between quotes rather than swallowing it.
    unrecognized: Option<String>,
}

/// What the on-disk file is, relative to the template about to be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileVerdict {
    /// The file does not exist — the `identity_toml_absent` population.
    Absent,
    /// Byte-for-byte equal to the template: nothing to write, nothing to back up.
    Identical,
    /// Present and different: it is backed up, then replaced.
    Differs,
}

impl FileVerdict {
    fn needs_write(self) -> bool {
        !matches!(self, Self::Identical)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Absent => "absent      → will be created",
            Self::Identical => "identical   → left untouched",
            Self::Differs => "differs     → backed up, then replaced",
        }
    }
}

/// One file of the plan: its path, the content to apply, and the verdict.
struct PlannedFile {
    name: &'static str,
    path: PathBuf,
    expected: String,
    verdict: FileVerdict,
}

/// Re-apply the authoritative identity (and persona) templates for `name`.
///
/// `out` is written rather than `println!`ed so the whole report — pre-digest,
/// warnings, outcome — is assertable by the tests without a terminal.
fn reprovision(
    global_home: &Path,
    name: &str,
    opts: ReprovisionOptions<'_>,
    out: &mut dyn Write,
) -> Result<()> {
    let name = agent::normalize_agent_name(name);
    agent::validate_agent_name(&name)?;

    // Population predicate (D5). NOT `agent::agent_exists`, which tests
    // `config.toml`: this verb exists to repair agent homes where one of the two
    // files is missing, and the mika#2027 shape — `identity.toml` gone,
    // `config.toml` present — is invisible to every narrower predicate. The
    // server's own definition of "an agent I could serve" is the union, and it
    // has exactly one reader.
    let servable = mika_agent::server::tier_guard::servable_agent_names(global_home);
    if !servable.iter().any(|n| n == &name) {
        bail!(
            "Agent '{name}' not found under {}. \
             Create it with `mika agents create {name}` — this command re-applies \
             an existing agent's templates, it does not create agents.",
            global_home.join("agents").display()
        );
    }

    let agent_home = agent::agent_dir(global_home, &name);
    let (identity, soul, source_line, resolved_tier) =
        render_sources(global_home, &agent_home, &name, opts.tier)?;

    // D4 — fail closed on the rendering, before anything is written.
    validate_rendered_identity(&identity)?;

    let mut plan = vec![PlannedFile {
        name: "identity.toml",
        verdict: verdict_for(&agent_home.join("identity.toml"), &identity)?,
        path: agent_home.join("identity.toml"),
        expected: identity,
    }];
    let soul_path = agent_home.join("soul.md");
    let soul_verdict = if opts.identity_only {
        // Scoped away from this file by the operator's own flag, so an unreadable
        // persona must not abort a repair that was never going to touch it.
        // Unknown reads as "diverges", which only arms the warning below.
        verdict_for(&soul_path, &soul).unwrap_or(FileVerdict::Differs)
    } else {
        verdict_for(&soul_path, &soul)?
    };
    if !opts.identity_only {
        plan.push(PlannedFile {
            name: "soul.md",
            verdict: soul_verdict,
            path: soul_path,
            expected: soul,
        });
    }

    // Pre-digest.
    writeln!(out, "\n  Re-provision agent '{name}'")?;
    writeln!(out, "    source: {source_line}")?;
    writeln!(out, "    home:   {}", agent_home.display())?;
    writeln!(out)?;
    for file in &plan {
        writeln!(out, "    {:<14} {}", file.name, file.verdict.label())?;
    }
    if opts.identity_only {
        writeln!(out, "    {:<14} SKIPPED (--identity-only)", "soul.md")?;
    }

    if opts.identity_only && soul_verdict.needs_write() {
        // D1 — `--identity-only` is an escape hatch with a warning, not a silence.
        // Leaving the persona behind while moving the allowlist puts the two
        // detection axes of the mika#1962 guard into disagreement, which is
        // precisely the state that guard reports. Saying so costs one line here;
        // discovering it costs a refused startup later.
        writeln!(
            out,
            "\n  WARNING — --identity-only leaves the two tier axes in disagreement.\n\
             \x20   soul.md on disk is not the persona this template prescribes, so the\n\
             \x20   skill allowlist (axis 2) and the persona marker (axis 1) will not\n\
             \x20   agree. That is the exact state the mika#1962 boot guard reports.\n\
             \x20   Re-run without --identity-only to move both axes together."
        )?;
    }

    if let Some(tier) = resolved_tier
        && tier.expects_family_provisioning()
    {
        // D3 — this is not a per-agent refusal. `assert_family_tier_env_consistency`
        // `bail!`s out of `run_server`, so a family-provisioned agent under a
        // non-family process tier takes EVERY agent down at the next restart.
        writeln!(
            out,
            "\n  BEFORE RESTARTING — this writes family-tier provisioning to disk.\n\
             \x20   mika-spirit refuses to start when an agent is family-provisioned\n\
             \x20   while the PROCESS tier is not (mika#1962), and that refusal is\n\
             \x20   process-wide: every other agent goes down with it.\n\
             \x20   Set MIKA_AGENT_TIER=family in the SERVICE environment — the\n\
             \x20   EnvironmentFile, the systemd drop-in, or the K8s ConfigMap —\n\
             \x20   never in an interactive shell."
        )?;
    } else if let Some(tier) = resolved_tier
        && !tier.expects_family_provisioning()
        && reads_as_family_provisioned(&agent_home)
    {
        // The reverse direction, named because it cannot be guarded — the
        // mika#1962 guard detects *family* provisioning only, so writing an
        // operator template over a family agent removes the markers and nothing
        // will ever say so. Conditioned on the agent ACTUALLY reading as family
        // today: a note printed on every operator re-provision is a note
        // operators learn to skip, and this one has to be read.
        writeln!(
            out,
            "\n  WARNING — this agent currently reads as FAMILY-provisioned, and you\n\
             \x20   are writing a {tier:?}-tier template over it.\n\
             \x20   This direction is NOT guarded: the mika#1962 startup guard only\n\
             \x20   detects family provisioning, so once the markers are gone nothing\n\
             \x20   will report the mismatch. If the service still runs with\n\
             \x20   MIKA_AGENT_TIER=family, this agent will be served under operator\n\
             \x20   semantics, silently. Change the service environment too."
        )?;
    }

    if !plan.iter().any(|f| f.verdict.needs_write()) {
        writeln!(
            out,
            "\n  Nothing to do — every file already matches the template.\n"
        )?;
        return Ok(());
    }

    if opts.dry_run {
        writeln!(out, "\n  Dry run — nothing was written.\n")?;
        return Ok(());
    }

    if !opts.yes {
        if !io::stdin().is_terminal() {
            bail!(
                "Non-interactive terminal requires --yes flag to bypass confirmation. \
                 Use: mika agents reprovision {name} --yes"
            );
        }
        write!(out, "\n  Type the agent name to confirm: ")?;
        out.flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if input.trim() != name {
            writeln!(out, "  Name mismatch — aborting.")?;
            return Ok(());
        }
    }

    writeln!(out)?;
    for file in &plan {
        if !file.verdict.needs_write() {
            continue;
        }
        if file.verdict == FileVerdict::Differs {
            let backup = backup_path(&file.path);
            std::fs::copy(&file.path, &backup)
                .with_context(|| format!("backing up {}", file.path.display()))?;
            set_owner_only(&backup)?;
            writeln!(out, "    backup  {}", backup.display())?;
        }
        write_atomic_owner_only(&file.path, &file.expected)?;
        writeln!(out, "    wrote   {}", file.path.display())?;
    }

    // R9 — self-check against the tier THIS process resolves. The other half of
    // the question is the service's environment, which a CLI structurally cannot
    // read; the block above is what covers it.
    let process_tier = resolved_tier.unwrap_or_else(AgentTier::from_env);
    match mika_agent::server::tier_guard::check_agent_tier_consistency(
        &agent_home,
        &name,
        process_tier,
    ) {
        Ok(()) => writeln!(
            out,
            "\n  Self-check: on-disk provisioning agrees with the tier this process \
             resolves ({process_tier:?})."
        )?,
        // Deliberately NOT rolled back: the disk is now coherent with the tier
        // that was asked for; it is the *process* that is not. The guard's own
        // message already names the agent and the fix, so it is shown verbatim.
        Err(e) => writeln!(out, "\n  Self-check FAILED:\n    {e}")?,
    }

    writeln!(
        out,
        "\n  Next:\n\
         \x20   1. restart mika-spirit (rc-service / systemctl / kubectl rollout restart)\n\
         \x20   2. grep -E 'identity_toml_(absent|unreadable|malformed)' \"$MIKA_SPIRIT_LOG_FILE\"\n\
         \x20      — must return nothing new\n\
         \x20   3. mika skills list --agent {name}\n\
         \x20      — reads identity.toml live from disk, so it shows the repaired\n\
         \x20        boundary immediately, BEFORE any restart\n\n\
         \x20 The symlinks under {} are re-materialized by the next startup, not by\n\
         \x20 this command. An empty-looking directory before the restart is expected;\n\
         \x20 do not reinstall skills to \"repair\" it.\n",
        agent_home.join("skills").display()
    )?;

    Ok(())
}

/// Resolve which templates apply to `name`, and under which tier.
///
/// Returns `(identity, soul, source_line, tier)`. The tier is `None` for a
/// well-known agent: its identity derives from its spec, not from a tier, which
/// is why `--tier` is an error there rather than a no-op (D2 — a silently ignored
/// flag leaves the operator believing they posed something).
#[allow(clippy::type_complexity)]
fn render_sources(
    global_home: &Path,
    agent_home: &Path,
    name: &str,
    tier_arg: Option<&str>,
) -> Result<(String, String, String, Option<AgentTier>)> {
    if let Some(spec) = mika_agent::well_known_agents::find_well_known_agent(name) {
        if tier_arg.is_some() {
            bail!(
                "Agent '{name}' is a well-known agent: its identity comes from its \
                 code spec, not from a tier, so --tier has nothing to apply. \
                 Re-run without --tier."
            );
        }
        // `load_for_agent` rather than `load`: this is a per-agent operation, and
        // mika-arch's computed identity reads `kg_docs_roots`, which a per-agent
        // config.toml may legitimately carry.
        let settings = mika_common::config::Settings::load_for_agent(global_home, agent_home)
            .with_context(|| format!("loading settings for agent '{name}'"))?;
        // D4 (1) — an `Err` here is mika-arch without `MIKA_KG_DOCS_ROOTS`.
        // Writing anyway would produce an architect with no corpus: an agent that
        // starts, answers, and finds nothing.
        let identity = mika_agent::well_known_agents::render_identity_content(spec, &settings)
            .map_err(|e| {
                anyhow!(
                    "refusing to write {}: the authoritative identity for '{name}' \
                     could not be rendered — {e}",
                    agent_home.join("identity.toml").display()
                )
            })?;
        return Ok((
            identity,
            spec.soul.to_string(),
            format!("well-known spec '{}'", spec.name),
            None,
        ));
    }

    let resolved = resolve_tier_arg(tier_arg);
    let mut source = format!("tier {:?} (via {})", resolved.tier, resolved.provenance);
    if let Some(raw) = &resolved.unrecognized {
        source.push_str(&format!(
            " — value \"{raw}\" not recognized, failed closed to {:?} (mika#2023)",
            resolved.tier
        ));
    }
    Ok((
        resolved.tier.identity_toml().to_string(),
        resolved.tier.soul_md().to_string(),
        source,
        Some(resolved.tier),
    ))
}

/// Does this agent home read as family-provisioned **today**, on either of the
/// two mika#1962 detection axes?
///
/// Fail-**open** (an unreadable file answers `false`), deliberately, and the
/// asymmetry with the guard it mirrors is the point: the guard refuses a startup,
/// so it must err towards detecting; this only decides whether to print a
/// warning, so erring towards a false alarm would train the operator to skip the
/// one line that matters. A genuine drift the guard can see is still reported by
/// the guard, at startup, where it is enforceable.
fn reads_as_family_provisioned(agent_home: &Path) -> bool {
    home::soul_has_family_marker(agent_home).unwrap_or(false)
        || home::identity_allowlist_matches_family(agent_home).unwrap_or(false)
}

/// `--tier` when given, else `MIKA_AGENT_TIER`, else the default — and always
/// say which.
///
/// The parsing itself is [`AgentTier::parse`]'s and nothing else's: the tier
/// vocabulary and its fail-closed rule have one reader (mika#2230 D2, guarded by
/// `mika2230_le_tier_a_un_seul_analyseur` in `mika-common`).
fn resolve_tier_arg(flag: Option<&str>) -> ResolvedTier {
    if let Some(raw) = flag {
        let parsed = AgentTier::parse(raw);
        return ResolvedTier {
            tier: parsed.tier,
            provenance: "--tier".to_string(),
            unrecognized: (!parsed.recognized).then(|| raw.to_string()),
        };
    }
    match std::env::var("MIKA_AGENT_TIER") {
        Ok(raw) => {
            let parsed = AgentTier::parse(&raw);
            ResolvedTier {
                tier: parsed.tier,
                provenance: "MIKA_AGENT_TIER".to_string(),
                unrecognized: (!parsed.recognized).then(|| raw.clone()),
            }
        }
        Err(_) => ResolvedTier {
            tier: AgentTier::Default,
            provenance: "default (MIKA_AGENT_TIER unset)".to_string(),
            unrecognized: None,
        },
    }
}

/// D4 (2) and (3) — refuse to write an identity that is not valid TOML, or whose
/// `[skills].allowlist` is absent or empty.
///
/// The empty case is the trap the runbook writes in a callout:
/// `apply_identity_allowlist` returns early on an empty list, so `allowlist = []`
/// means *no filter* — every bundled skill active, `shell-exec` included. A
/// repair tool able to write the most permissive configuration in the system
/// without saying so would be worse than the hand gesture it replaces.
fn validate_rendered_identity(content: &str) -> Result<()> {
    let value: toml::Value = toml::from_str(content).map_err(|e| {
        anyhow!(
            "refusing to write identity.toml: the rendered template is not valid \
             TOML ({e}). This is a defect in the template itself, not in the agent \
             on disk — nothing was written."
        )
    })?;

    match value
        .get("skills")
        .and_then(|s| s.get("allowlist"))
        .and_then(|a| a.as_array())
    {
        Some(list) if !list.is_empty() => Ok(()),
        Some(_) => bail!(
            "refusing to write identity.toml: the rendered template carries an \
             EMPTY `[skills].allowlist`. An empty allowlist is not a restriction — \
             `apply_identity_allowlist` returns early on it, so every bundled skill \
             stays active, `shell-exec` and `git-ops` included. Nothing was written."
        ),
        None => bail!(
            "refusing to write identity.toml: the rendered template carries no \
             `[skills].allowlist`. An absent allowlist is default-permissive \
             (mika#1596) — it grants every bundled skill. Nothing was written."
        ),
    }
}

/// Compare the on-disk file with the template about to be applied.
///
/// An **unreadable** file is an error, never a `Differs`: `identity_toml_unreadable`
/// is a permissions or I/O fault on content that is still there, and the runbook's
/// § 4 says in as many words not to re-provision over it.
fn verdict_for(path: &Path, expected: &str) -> Result<FileVerdict> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(if bytes == expected.as_bytes() {
            FileVerdict::Identical
        } else {
            FileVerdict::Differs
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(FileVerdict::Absent),
        Err(e) => Err(anyhow!(
            "refusing to re-provision: {} exists but cannot be read ({e}). \
             Its content is still there — fix ownership/permissions instead of \
             overwriting it (see docs/operator/agent-identity-reprovision.md § 4).",
            path.display()
        )),
    }
}

/// `<file>.bak.<YYYYMMDDTHHMMSSZ>`, beside the file it copies.
fn backup_path(path: &Path) -> PathBuf {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let mut name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("file"))
        .to_os_string();
    name.push(format!(".bak.{stamp}"));
    path.with_file_name(name)
}

/// Write via `.tmp` + `rename`, the same shape `reconcile_well_known_identity`
/// uses, so a crash mid-write never leaves a partial identity on disk.
fn write_atomic_owner_only(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("reprovision.tmp");
    std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    set_owner_only(&tmp)?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming onto {}", path.display()))?;
    set_owner_only(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 600 {}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path, depth: u32) -> Result<()> {
    if depth > 10 {
        bail!("directory nesting too deep while copying {}", src.display());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            continue;
        }
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path, depth + 1)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn render(record: &BudgetRecord) -> String {
        let mut out: Vec<u8> = Vec::new();
        render_budget_text(record, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn sample() -> BudgetRecord {
        BudgetRecord {
            agent_id: "mika-arch".to_string(),
            http_timeout_secs: 240,
            agent_total_timeout_secs: 900,
            max_attempts: 3,
            effective_max_attempts: 3,
            retry_reachable: true,
            llm_max_tokens: 32_768,
            reachable_output_tokens: 9_000,
            http_source: "agent_config".to_string(),
            total_source: "agent_config".to_string(),
            max_tokens_source: "agent_config".to_string(),
            provider: "openrouter".to_string(),
            provider_source: "agent_config".to_string(),
            model: "moonshotai/kimi-k2.5".to_string(),
            model_source: "agent_config".to_string(),
            model_config_key: "openrouter_model".to_string(),
            resolved_at: "2026-09-21T06:12:44Z".to_string(),
            // The mika#2473 default is the honest one: a sample says nothing
            // about the drift until a test sets it, and "nothing said" renders
            // as *non évaluée*, never as *en phase*.
            model_drift: None,
        }
    }

    /// mika#2457 U4 — the attested render carries every fact the § 6 reading
    /// table branches on, **including the date**.
    ///
    /// The probe exists to separate three worlds (`agent_config` / `process_env`
    /// / `default`), so a render that dropped a provenance would leave the
    /// operator with the values and none of the three remedies. `resolved_at` is
    /// asserted for the reason F3 names: a record nobody can date cannot be
    /// compared against the `config.toml` mtime, and the comparison *is* what
    /// makes the drift decidable.
    #[test]
    fn mika2457_the_attested_render_carries_the_values_and_their_provenance() {
        let text = render(&sample());

        assert!(text.contains("mika-arch"));
        assert!(
            text.contains("2026-09-21T06:12:44Z"),
            "la sortie doit dire QUAND elle a été vraie : {text}"
        );
        assert!(text.contains("240 s"), "plafond absent : {text}");
        assert!(text.contains("900 s"), "enveloppe absente : {text}");
        assert!(text.contains("32768"), "max_tokens absent : {text}");
        assert!(
            text.contains("moonshotai/kimi-k2.5"),
            "modèle absent : {text}"
        );
        assert!(
            text.contains("openrouter_model"),
            "la clé qu'un opérateur éditerait doit être nommée : {text}"
        );
        assert!(
            text.matches("agent_config").count() >= 4,
            "chaque fait porte sa provenance — c'est ce qui sépare les trois \
             mondes du § 6 : {text}"
        );
    }

    /// mika#2457 U4 (non-régression) — an unreadable provider renders as such,
    /// never as an empty model.
    ///
    /// `unknown_provider` is **pre-existing** behaviour (`budget_provenance.rs`,
    /// mika#2328); this ticket introduces no new refusal. What is asserted here
    /// is only that the provenance survives the trip through the record and the
    /// route. On that crossing `effective_model()` is `None`, so the field
    /// arrives empty — and printing an empty value would read as "no model
    /// declared", which is not what is known. What is known is that the provider
    /// could not be parsed, and the raw string that failed is the actionable
    /// half.
    #[test]
    fn mika2457_an_unreadable_provider_is_rendered_as_unresolved_not_as_empty() {
        let mut record = sample();
        record.provider = "zorglub".to_string();
        record.model = String::new();
        record.model_source = MODEL_SOURCE_UNKNOWN_PROVIDER.to_string();
        record.model_config_key = String::new();

        let text = render(&record);
        assert!(
            text.contains("non résolu") && text.contains("zorglub"),
            "le brut qui a échoué à parser est la moitié actionnable : {text}"
        );
        assert!(
            !text.contains("model      \n") && !text.contains("model       ("),
            "un modèle vide se lirait « aucun modèle déclaré », ce qui est \
             inconnu et possiblement faux : {text}"
        );
    }

    /// mika#2457 U4 — the retry-unreachable geometry is surfaced, not silently
    /// rendered as nominal.
    #[test]
    fn mika2457_an_unreachable_retry_is_named_in_the_render() {
        let mut record = sample();
        record.max_attempts = 2;
        record.effective_max_attempts = 1;
        record.retry_reachable = false;

        let text = render(&record);
        assert!(
            text.contains("mika#2362"),
            "une géométrie dont la dernière tentative est inatteignable doit \
             le dire : {text}"
        );
    }

    /// mika#2457 U4 **negative control** — an unreachable server prints "not
    /// attested" and **no value**.
    ///
    /// This is the control the whole unit rests on. Without it, "the CLI reads
    /// the server" and "the CLI computes locally" produce the *same* output on
    /// a workstation where both processes share an environment — which is
    /// exactly the machine this test would be written on. Asserting that the
    /// unreachable path prints no number is what makes the difference
    /// observable at all.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2457_an_unreachable_server_attests_nothing_and_no_local_value() {
        // Safety: test-only env vars, serialized by `#[serial]`.
        unsafe {
            // Port 1 is reserved and never listening — the "daemon is down" case.
            std::env::set_var("MIKA_SPIRIT_URL", "http://127.0.0.1:1");
            // Well-formed (64 hex): `#[serial]` does not fence unmarked tests
            // that call `Settings::load`, which rejects a malformed token.
            std::env::set_var("MIKA_INTERNAL_TOKEN", "ab".repeat(32));
        }

        let mut out: Vec<u8> = Vec::new();
        budget("mika-arch", &OutputFormat::Text, &mut out)
            .await
            .expect("an unreachable server is an answer, not an error");
        let text = String::from_utf8(out).unwrap();

        assert!(
            text.contains("non attesté"),
            "le serveur n'a rien attesté, et c'est ce qu'il faut dire : {text}"
        );
        // The values a local resolution would have produced on this machine.
        for forbidden in ["120", "300", "240", "900", "agent_config", "process_env"] {
            assert!(
                !text.contains(forbidden),
                "aucune valeur résolue localement ne doit être affichée — \
                 « {forbidden} » est apparu : {text}"
            );
        }

        // JSON keeps the same contract: a flag, never a fabricated record.
        let mut out: Vec<u8> = Vec::new();
        budget("mika-arch", &OutputFormat::Json, &mut out)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(json["attested"], false);
        assert!(
            json["http_timeout_secs"].is_null() && json["model"].is_null(),
            "un record non attesté ne porte aucune valeur : {json}"
        );

        unsafe {
            std::env::remove_var("MIKA_SPIRIT_URL");
            std::env::remove_var("MIKA_INTERNAL_TOKEN");
        }
    }

    /// mika#2457 — a server that ANSWERS without attesting is not told apart
    /// by "no values" alone: the render names the cause.
    ///
    /// The negative control above only drives connection-refused. A `401` (a
    /// stale `MIKA_INTERNAL_TOKEN`), a `5xx`, and a 2xx body the mirror cannot
    /// read used to share its sentence — "unreachable, predating the fix, or not
    /// serving this agent" — which is a false diagnosis for all three. Each case
    /// here must still print no value, and must say which answer it got.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2457_a_non_2xx_or_unreadable_answer_names_its_cause() {
        use axum::{Router, http::StatusCode, routing::get};

        let app = Router::new()
            .route(
                "/api/v1/agents/denied/budget",
                get(|| async { (StatusCode::UNAUTHORIZED, "{\"error\":\"unauthorized\"}") }),
            )
            .route(
                "/api/v1/agents/broken/budget",
                get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
            )
            .route(
                "/api/v1/agents/skewed/budget",
                get(|| async { (StatusCode::OK, "{\"budget\":{\"agent_id\":\"skewed\"}}") }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app.into_make_service())
                .await
                .unwrap();
        });

        // Safety: test-only env vars, serialized by `#[serial]`.
        unsafe {
            std::env::set_var("MIKA_SPIRIT_URL", format!("http://{addr}"));
            std::env::set_var("MIKA_INTERNAL_TOKEN", "ab".repeat(32));
        }

        for (agent, needle, reason, status) in [
            ("denied", "MIKA_INTERNAL_TOKEN", "unauthorized", Some(401)),
            ("broken", "a répondu 500", "http_error", Some(500)),
            ("ghost", "a répondu 404", "not_found", Some(404)),
            ("skewed", "illisible", "malformed_response", None),
        ] {
            let mut out: Vec<u8> = Vec::new();
            budget(agent, &OutputFormat::Text, &mut out).await.unwrap();
            let text = String::from_utf8(out).unwrap();
            assert!(text.contains("non attesté"), "{agent} : {text}");
            assert!(
                text.contains(needle),
                "{agent} : la cause réelle doit être nommée ({needle}) : {text}"
            );
            if agent != "ghost" {
                assert!(
                    !text.contains("antérieur au correctif"),
                    "{agent} : le diagnostic 404 ne doit pas couvrir une autre \
                     réponse : {text}"
                );
            }

            let mut out: Vec<u8> = Vec::new();
            budget(agent, &OutputFormat::Json, &mut out).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(json["attested"], false, "{agent} : {json}");
            assert_eq!(json["reason"], reason, "{agent} : {json}");
            assert_eq!(json["status"].as_u64(), status, "{agent} : {json}");
            assert!(
                json["http_timeout_secs"].is_null() && json["model"].is_null(),
                "{agent} : un record non attesté ne porte aucune valeur : {json}"
            );

            let mut out: Vec<u8> = Vec::new();
            budget(agent, &OutputFormat::Yaml, &mut out).await.unwrap();
            let yaml: serde_json::Value =
                serde_yaml::from_slice(&out).expect("the YAML render must parse");
            assert_eq!(yaml["reason"], reason, "{agent} (yaml) : {yaml}");
            assert_eq!(yaml["attested"], false, "{agent} (yaml) : {yaml}");
        }

        server.abort();
        unsafe {
            std::env::remove_var("MIKA_SPIRIT_URL");
            std::env::remove_var("MIKA_INTERNAL_TOKEN");
        }
    }

    /// mika#2457 U4 — **the CLI resolves no budget locally**, structurally.
    ///
    /// The behavioural control above proves *that* output came from the server.
    /// It cannot see a second reader added six months from now on another CLI
    /// path: that regression would make no decision wrong, it would make the
    /// guarantee inoperative in silence, and every behavioural assertion would
    /// stay green. That is the class
    /// `mika1883_run_usage_accumulates_only_via_the_one_helper` and
    /// `mika2205_periodic_scans_do_not_read_the_pat_field_directly` had to close
    /// with a source scan, for exactly this reason.
    ///
    /// **Allowlist shipped EMPTY, and measured empty at HEAD.** When this test
    /// fires, the resolution is **removed**, never allowlisted: an allowlist
    /// that stops being empty here *is* the mika#2304 false green reintroduced —
    /// the CLI printing a locally computed value with the authority of an
    /// attestation.
    #[test]
    fn mika2457_the_cli_resolves_no_budget_locally() {
        const ALLOWED: &[&str] = &[];

        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));

        // Each needle is a way to resolve a budget in this process rather than
        // read one the server resolved.
        let needles = [
            "BudgetProvenance",
            "ModelProvenance",
            "resolve_llm_budget_record",
            "effective_budget",
            "effective_llm_http_timeout_secs",
            "http_timeout_secs()",
            "llm_timeout_budget",
        ];

        let mut violations: Vec<String> = Vec::new();
        scanner.for_each(|path, source| {
            let rel = path
                .strip_prefix(scanner.src_root())
                .unwrap_or(path)
                .display()
                .to_string();
            if ALLOWED.contains(&rel.as_str()) {
                return;
            }
            for (idx, line) in source.lines().enumerate() {
                // A comment naming the rule is not a violation of it — the prose
                // above `budget()` names these symbols precisely in order to say
                // they are NOT called here, and a guard counting that mention
                // would forbid explaining itself.
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for needle in needles {
                    if line.contains(needle) {
                        violations.push(format!("{rel}:{} — {}", idx + 1, line.trim()));
                    }
                }
            }
        });

        assert!(
            violations.is_empty(),
            "le CLI ne doit résoudre aucun budget localement : spirit atteste, le \
             CLI rend. Quand ce scan tire, on RETIRE le second lecteur, on ne \
             l'allowliste pas — une allowlist qui cesse d'être vide ici est le \
             faux vert mika#2304 réintroduit.\n{}",
            violations.join("\n")
        );
    }

    /// The guard above must actually look at something — a scanner that found
    /// no file would make it vacuous and green for ever.
    #[test]
    fn mika2457_the_local_resolution_scan_is_not_vacuous() {
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let files = scanner.files();
        assert!(
            files.len() > 10,
            "le scan doit voir le crate, sinon il est vide de sens : {} fichier(s)",
            files.len()
        );
        assert!(
            files.iter().any(|p| p.ends_with("commands/agents.rs")),
            "le scan doit voir le fichier qui porte la commande"
        );
    }

    /// The `code` line of a render — the mika#2473 surface, isolated.
    fn code_line(text: &str) -> String {
        text.lines()
            .find(|l| l.trim_start().starts_with("code "))
            .unwrap_or_else(|| panic!("aucune ligne `code` dans :\n{text}"))
            .to_string()
    }

    fn drift_of(status: &str) -> ModelDrift {
        ModelDrift {
            status: status.to_string(),
            declared_provider: Some("openrouter".to_string()),
            declared_model: Some("moonshotai/kimi-k2.5".to_string()),
            runtime_provider: None,
            runtime_model: None,
            runtime_model_source: None,
        }
    }

    /// mika#2473 U5 — the render carries the four states the guard can be in,
    /// and the fourth is never folded into the second.
    ///
    /// The state this test exists for is the **absent key**: a spirit predating
    /// mika#2473 answers `{budget}` with no `model_drift`, and rendering that
    /// silence as *en phase* would attest a comparison nobody ran — mika#2457's
    /// *not attested* population, one field over. Hence the negative control at
    /// the end: the words *en phase* may appear on the `in_sync` render and
    /// nowhere else, which is the only assertion that makes the four states
    /// distinguishable rather than merely present.
    #[test]
    fn mika2473_the_render_carries_the_four_drift_states() {
        // 1. `drift` — the declared model, the one actually served, AND the
        //    door it came through. `process_env` rather than `agent_config` on
        //    purpose: the sample already says `agent_config` five times, so
        //    asserting that word would pass on a render that dropped the door.
        let mut record = sample();
        record.model_drift = Some(ModelDrift {
            runtime_provider: Some("openrouter".to_string()),
            runtime_model: Some("moonshotai/kimi-k3".to_string()),
            runtime_model_source: Some("process_env".to_string()),
            ..drift_of("drift")
        });
        let drift = render(&record);
        let line = code_line(&drift);
        assert!(
            line.contains("moonshotai/kimi-k2.5"),
            "le modèle déclaré par le dépôt doit être nommé : {line}"
        );
        assert!(
            line.contains("moonshotai/kimi-k3"),
            "le modèle réellement servi doit être nommé : {line}"
        );
        assert!(
            line.contains("process_env"),
            "la porte par laquelle le runtime a pris son modèle est la moitié \
             actionnable : {line}"
        );

        // The `code` line reads under the `model` line it qualifies.
        let order: Vec<&str> = drift
            .lines()
            .filter(|l| l.trim_start().starts_with("model ") || l.trim_start().starts_with("code "))
            .collect();
        assert_eq!(
            order.len(),
            2,
            "une ligne `model` et une ligne `code`, pas plus : {drift}"
        );
        assert!(
            order[0].trim_start().starts_with("model "),
            "la ligne `code` suit la ligne `model` : {drift}"
        );

        // 2. `in_sync` — and no door promised, because the arm carries none.
        let mut record = sample();
        record.model_drift = Some(drift_of("in_sync"));
        let in_sync = render(&record);
        let line = code_line(&in_sync);
        assert!(
            line.contains("en phase") && line.contains("moonshotai/kimi-k2.5"),
            "en phase, avec le modèle que le dépôt déclare : {line}"
        );

        // 3. `not_applicable` — nothing declared, so nothing to be out of phase
        //    with. A distinct sentence, never a silent `in_sync`.
        let mut record = sample();
        record.model_drift = Some(ModelDrift {
            declared_provider: None,
            declared_model: None,
            ..drift_of("not_applicable")
        });
        let not_applicable = render(&record);
        assert!(
            code_line(&not_applicable).contains("aucun modèle déclaré"),
            "un agent sans constante le dit : {not_applicable}"
        );

        // 4. Key absent — a spirit older than this fix. It did not evaluate;
        //    it did not find agreement.
        let mut record = sample();
        record.model_drift = None;
        let unevaluated = render(&record);
        let line = code_line(&unevaluated);
        assert!(
            line.contains("non évaluée"),
            "une clé absente est une garde qui n'a pas tourné : {line}"
        );
        assert!(
            !line.contains("aucun modèle déclaré"),
            "« pas évalué » et « rien de déclaré » sont deux états : {line}"
        );

        // NEGATIVE CONTROL — the whole point of the four states. If *en phase*
        // can be printed about a runtime nobody compared, the other three
        // renders are decoration.
        for (etat, texte) in [
            ("drift", &drift),
            ("not_applicable", &not_applicable),
            ("clé absente", &unevaluated),
        ] {
            assert!(
                !texte.contains("en phase"),
                "« en phase » n'appartient qu'à in_sync — il est apparu sur \
                 l'état {etat} :\n{texte}"
            );
        }
    }

    /// mika#2473 U5 — a status word this CLI does not know is not *en phase*
    /// either.
    ///
    /// The mirror below is a mirror precisely so a newer spirit can serve a
    /// fifth arm without breaking this render (the reason `BudgetRecord` is one
    /// too). A `match` whose catch-all fell through to the `in_sync` sentence
    /// would turn that tolerance into the false green this unit exists to
    /// close.
    #[test]
    fn mika2473_an_unknown_status_is_not_rendered_in_sync() {
        let mut record = sample();
        record.model_drift = Some(drift_of("recalibrating"));
        let text = render(&record);
        assert!(
            !text.contains("en phase"),
            "un mot inconnu n'est pas une mise en phase : {text}"
        );
        assert!(
            code_line(&text).contains("recalibrating"),
            "le mot reçu est dit, pour que l'opérateur sache quoi chercher : {text}"
        );
    }

    /// mika#2473 U5 — the structured formats carry the sibling, not only the
    /// text one.
    ///
    /// R7 names JSON **and** YAML; both are asserted here because they are two
    /// serializers over one value, and a field reaching one and missing the
    /// other is the shape `render_unattested` already keeps one payload to
    /// avoid.
    #[test]
    fn mika2473_json_output_carries_model_drift() {
        let mut record = sample();
        record.model_drift = Some(ModelDrift {
            runtime_provider: Some("openrouter".to_string()),
            runtime_model: Some("moonshotai/kimi-k3".to_string()),
            runtime_model_source: Some("process_env".to_string()),
            ..drift_of("drift")
        });

        let json: serde_json::Value = serde_json::to_value(&record).unwrap();
        assert_eq!(json["model_drift"]["status"], "drift", "{json}");
        assert_eq!(json["model_drift"]["runtime_model"], "moonshotai/kimi-k3");
        assert_eq!(json["model_drift"]["runtime_model_source"], "process_env");

        let yaml = serde_yaml::to_string(&record).unwrap();
        let back: serde_json::Value = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back["model_drift"]["status"], "drift", "{yaml}");
    }

    /// mika#2473 U5 — the key is read off the **response**, tolerantly, and a
    /// spirit that does not serve it is not rendered *en phase*.
    ///
    /// The two renders above start from a record built in the test. This one
    /// starts from the wire, because the tolerance R7 asks for lives in
    /// `budget()`'s read and nowhere else: `model_drift` is a sibling of
    /// `budget`, not a field of it, so a reader that deserialized only
    /// `v["budget"]` would produce `None` for **every** spirit — including the
    /// ones that answer correctly — and the four states would collapse to one
    /// that happens to be honest.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2473_the_drift_is_read_off_the_response_beside_the_budget() {
        use axum::{Router, routing::get};

        let mut budget_json = serde_json::to_value(sample()).unwrap();
        budget_json
            .as_object_mut()
            .unwrap()
            .remove("model_drift")
            .expect("le record d'échantillon porte le champ");

        let served = budget_json.clone();
        let app = Router::new()
            .route(
                "/api/v1/agents/moderne/budget",
                get(move || {
                    let body = serde_json::json!({
                        "budget": served,
                        "model_drift": {
                            "status": "drift",
                            "declared_provider": "openrouter",
                            "declared_model": "moonshotai/kimi-k2.5",
                            "runtime_provider": "openrouter",
                            "runtime_model": "moonshotai/kimi-k3",
                            "runtime_model_source": "process_env",
                            "runtime_provider_source": "agent_config",
                            "model_config_key": "openrouter_model",
                        },
                    });
                    async move { axum::Json(body) }
                }),
            )
            .route(
                // A spirit predating mika#2473: the budget, and no sibling.
                "/api/v1/agents/ancien/budget",
                get(move || {
                    let body = serde_json::json!({ "budget": budget_json });
                    async move { axum::Json(body) }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app.into_make_service())
                .await
                .unwrap();
        });

        // Safety: test-only env vars, serialized by `#[serial]`.
        unsafe {
            std::env::set_var("MIKA_SPIRIT_URL", format!("http://{addr}"));
            std::env::set_var("MIKA_INTERNAL_TOKEN", "ab".repeat(32));
        }

        let mut out: Vec<u8> = Vec::new();
        budget("moderne", &OutputFormat::Text, &mut out)
            .await
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("moonshotai/kimi-k3") && text.contains("process_env"),
            "la dérive servie à côté du record doit être rendue : {text}"
        );
        assert!(
            !text.contains("en phase"),
            "un runtime qui sert autre chose n'est pas en phase : {text}"
        );

        let mut out: Vec<u8> = Vec::new();
        budget("moderne", &OutputFormat::Json, &mut out)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(json["model_drift"]["status"], "drift", "{json}");

        let mut out: Vec<u8> = Vec::new();
        budget("ancien", &OutputFormat::Text, &mut out)
            .await
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("non évaluée"),
            "un spirit qui ne sert pas la clé n'a rien évalué : {text}"
        );
        assert!(
            !text.contains("en phase"),
            "et ne doit surtout pas passer pour « en phase » : {text}"
        );

        server.abort();
        unsafe {
            std::env::remove_var("MIKA_SPIRIT_URL");
            std::env::remove_var("MIKA_INTERNAL_TOKEN");
        }
    }
}

#[cfg(test)]
mod reprovision_tests {
    use super::*;
    use mika_common::home::{
        DEFAULT_IDENTITY, DEFAULT_SOUL, FAMILY_IDENTITY, FAMILY_SOUL, FAMILY_SOUL_MARKER,
        identity_allowlist_matches_family, soul_has_family_marker,
    };
    use tempfile::TempDir;

    const CONFIG: &str = "log_level = \"info\"\n";

    /// A multi-agent home with one agent directory, populated à la carte.
    fn home_with(
        name: &str,
        config: Option<&str>,
        identity: Option<&str>,
        soul: Option<&str>,
    ) -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("agents").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(c) = config {
            std::fs::write(dir.join("config.toml"), c).unwrap();
        }
        if let Some(i) = identity {
            std::fs::write(dir.join("identity.toml"), i).unwrap();
        }
        if let Some(s) = soul {
            std::fs::write(dir.join("soul.md"), s).unwrap();
        }
        tmp
    }

    fn opts<'a>(tier: Option<&'a str>) -> ReprovisionOptions<'a> {
        ReprovisionOptions {
            tier,
            identity_only: false,
            dry_run: false,
            yes: true,
        }
    }

    /// Run the verb and hand back what it wrote to its report stream.
    fn run(home: &Path, name: &str, o: ReprovisionOptions<'_>) -> Result<String> {
        let mut out: Vec<u8> = Vec::new();
        reprovision(home, name, o, &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }

    fn read(home: &Path, name: &str, file: &str) -> String {
        std::fs::read_to_string(home.join("agents").join(name).join(file)).unwrap()
    }

    fn backups(home: &Path, name: &str, prefix: &str) -> Vec<PathBuf> {
        let dir = home.join("agents").join(name);
        let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&format!("{prefix}.bak.")))
            })
            .collect();
        out.sort();
        out
    }

    // -- Population and refusals (V1–V4) -------------------------------------

    /// **V1** — an agent nobody can serve is refused, and the refusal names the
    /// verb that does create agents.
    #[test]
    fn mika2230_v1_unknown_agent_is_refused_and_names_create() {
        let home = home_with("nadia", Some(CONFIG), Some(DEFAULT_IDENTITY), None);
        let err = run(home.path(), "ghost", opts(Some("default"))).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ghost"), "must name the agent: {msg}");
        assert!(
            msg.contains("mika agents create"),
            "must name the creating verb: {msg}"
        );
    }

    /// **V2** — the measured mika#2027 shape: `config.toml` present,
    /// `identity.toml` gone. It must be *in* the population, because it is
    /// exactly what this verb repairs.
    #[test]
    fn mika2230_v2_agent_missing_only_its_identity_is_in_the_population() {
        let home = home_with("nadia", Some(CONFIG), None, Some(DEFAULT_SOUL));
        run(home.path(), "nadia", opts(Some("default"))).expect("must be accepted");
        assert_eq!(
            read(home.path(), "nadia", "identity.toml"),
            DEFAULT_IDENTITY
        );
    }

    /// **V2b** — the negative control that justifies reading the population from
    /// `tier_guard::servable_agent_names` (D5) rather than from
    /// `agent::agent_exists`: an agent home carrying `identity.toml` and **no**
    /// `config.toml` is fully servable and invisible to the narrower predicate.
    #[test]
    fn mika2230_v2b_agent_without_config_toml_is_still_in_the_population() {
        let home = home_with("nadia", None, Some(DEFAULT_IDENTITY), None);
        assert!(
            !agent::agent_exists(home.path(), "nadia"),
            "precondition: the narrower predicate really does miss it"
        );
        run(home.path(), "nadia", opts(Some("default"))).expect("must be accepted");
        assert_eq!(read(home.path(), "nadia", "soul.md"), DEFAULT_SOUL);
    }

    /// **V3** — `--tier` on a well-known agent is an error, not a no-op: its
    /// identity derives from a code spec, so accepting the flag would leave the
    /// operator believing they had posed something (D2).
    #[test]
    fn mika2230_v3_tier_flag_is_refused_for_a_well_known_agent() {
        let home = home_with("mika-dev", Some(CONFIG), Some(DEFAULT_IDENTITY), None);
        let err = run(home.path(), "mika-dev", opts(Some("family"))).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--tier"), "must name the flag: {msg}");
        assert!(msg.contains("mika-dev"), "must name the agent: {msg}");
    }

    /// **V4** — a non-interactive terminal without `--yes` is refused, and
    /// nothing on disk moved. A confirmation must never be answered on the
    /// operator's behalf.
    #[test]
    fn mika2230_v4_non_tty_without_yes_is_refused_and_writes_nothing() {
        let home = home_with("nadia", Some(CONFIG), Some("name = \"x\"\n"), None);
        let before = read(home.path(), "nadia", "identity.toml");
        let err = run(
            home.path(),
            "nadia",
            ReprovisionOptions {
                tier: Some("default"),
                identity_only: false,
                dry_run: false,
                yes: false,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("--yes"), "{err}");
        assert_eq!(read(home.path(), "nadia", "identity.toml"), before);
        assert!(
            !home.path().join("agents/nadia/soul.md").exists(),
            "soul.md must not have been created"
        );
        assert!(backups(home.path(), "nadia", "identity.toml").is_empty());
    }

    // -- Fail-closed rendering (V5–V7) ---------------------------------------

    /// **V5** — mika-arch's identity is *computed* and needs `MIKA_KG_DOCS_ROOTS`.
    /// A failed render refuses the write rather than producing an architect with
    /// no corpus: an agent that starts, answers, and finds nothing.
    #[test]
    #[serial_test::serial]
    fn mika2230_v5_a_failed_computed_render_refuses_the_write() {
        // Safety: serialized against every other test touching these vars.
        unsafe {
            std::env::remove_var("MIKA_KG_DOCS_ROOTS");
            std::env::remove_var("MIKA_KG_DOCS_ROOT");
        }
        let sentinel = "name = \"untouched\"\n";
        let home = home_with("mika-arch", Some(CONFIG), Some(sentinel), None);
        let err = run(home.path(), "mika-arch", opts(None)).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("MIKA_KG_DOCS_ROOTS"),
            "the refusal must name what is missing: {msg}"
        );
        assert_eq!(
            read(home.path(), "mika-arch", "identity.toml"),
            sentinel,
            "nothing may be written when the authoritative render failed"
        );
    }

    /// **V6** — the runbook's boxed trap, made structural. `allowlist = []` is
    /// *not* a restriction: `apply_identity_allowlist` returns early on it, so an
    /// empty list means every bundled skill, `shell-exec` included. A repair tool
    /// able to write the most permissive configuration in the system without
    /// saying so would be worse than the hand gesture it replaces.
    #[test]
    fn mika2230_v6_an_empty_allowlist_refuses_the_write() {
        let err =
            validate_rendered_identity("name = \"x\"\n\n[skills]\nallowlist = []\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("EMPTY"), "{msg}");
        assert!(msg.contains("Nothing was written"), "{msg}");
    }

    /// **V7** — same family: an absent `[skills]` section is default-permissive
    /// (mika#1596), so it is refused too.
    #[test]
    fn mika2230_v7_a_missing_allowlist_refuses_the_write() {
        let err = validate_rendered_identity("name = \"x\"\nemoji = \"y\"\n").unwrap_err();
        assert!(err.to_string().contains("no `[skills].allowlist`"), "{err}");
        // And the template the verb actually ships must pass, or the guard would
        // be refusing the nominal path.
        validate_rendered_identity(DEFAULT_IDENTITY).expect("DEFAULT_IDENTITY must pass");
        validate_rendered_identity(FAMILY_IDENTITY).expect("FAMILY_IDENTITY must pass");
    }

    // -- Writing (V8–V14) -----------------------------------------------------

    /// **V8** — the measured mika#2027 case: identity absent, persona intact and
    /// already equal to the template. Exactly one file is created, and the
    /// untouched one is neither rewritten nor backed up (R5 by construction, not
    /// by a branch).
    #[test]
    fn mika2230_v8_only_the_differing_file_is_written() {
        let home = home_with("nadia", Some(CONFIG), None, Some(DEFAULT_SOUL));
        let soul_before = std::fs::metadata(home.path().join("agents/nadia/soul.md"))
            .unwrap()
            .modified()
            .unwrap();

        run(home.path(), "nadia", opts(Some("default"))).unwrap();

        assert_eq!(
            read(home.path(), "nadia", "identity.toml"),
            DEFAULT_IDENTITY
        );
        assert_eq!(read(home.path(), "nadia", "soul.md"), DEFAULT_SOUL);
        assert!(
            backups(home.path(), "nadia", "soul.md").is_empty(),
            "an identical file is not backed up"
        );
        assert!(
            backups(home.path(), "nadia", "identity.toml").is_empty(),
            "an absent file has nothing to back up"
        );
        assert_eq!(
            std::fs::metadata(home.path().join("agents/nadia/soul.md"))
                .unwrap()
                .modified()
                .unwrap(),
            soul_before,
            "soul.md must not have been rewritten"
        );
    }

    /// **V9** — both files differ: both are replaced, both are backed up, and the
    /// backups carry the original bytes.
    #[test]
    fn mika2230_v9_both_files_are_backed_up_byte_for_byte() {
        let old_identity = "name = \"Old\"\n\n[skills]\nallowlist = [\"calendar\"]\n";
        let old_soul = "# an operator's hand-tuned persona\n";
        let home = home_with("nadia", Some(CONFIG), Some(old_identity), Some(old_soul));

        run(home.path(), "nadia", opts(Some("default"))).unwrap();

        assert_eq!(
            read(home.path(), "nadia", "identity.toml"),
            DEFAULT_IDENTITY
        );
        assert_eq!(read(home.path(), "nadia", "soul.md"), DEFAULT_SOUL);

        let ib = backups(home.path(), "nadia", "identity.toml");
        let sb = backups(home.path(), "nadia", "soul.md");
        assert_eq!(ib.len(), 1, "one identity backup: {ib:?}");
        assert_eq!(sb.len(), 1, "one soul backup: {sb:?}");
        assert_eq!(std::fs::read(&ib[0]).unwrap(), old_identity.as_bytes());
        assert_eq!(std::fs::read(&sb[0]).unwrap(), old_soul.as_bytes());
    }

    /// **V10** — idempotence: a second run writes nothing and backs up nothing.
    #[test]
    fn mika2230_v10_a_second_run_is_a_no_op() {
        let home = home_with("nadia", Some(CONFIG), Some("name = \"Old\"\n"), None);
        run(home.path(), "nadia", opts(Some("default"))).unwrap();
        for prefix in ["identity.toml", "soul.md"] {
            for stale in backups(home.path(), "nadia", prefix) {
                std::fs::remove_file(stale).unwrap();
            }
        }

        let report = run(home.path(), "nadia", opts(Some("default"))).unwrap();
        assert!(report.contains("Nothing to do"), "{report}");
        assert!(backups(home.path(), "nadia", "identity.toml").is_empty());
        assert!(backups(home.path(), "nadia", "soul.md").is_empty());
    }

    /// **V11** — `--identity-only` leaves `soul.md` strictly alone, and the cost
    /// is *stated*: the two detection axes of the mika#1962 guard end up in
    /// disagreement, which is the state that guard exists to report.
    #[test]
    fn mika2230_v11_identity_only_leaves_the_persona_and_says_so() {
        let hand_written = "# a persona the operator wrote\n";
        let home = home_with("nadia", Some(CONFIG), None, Some(hand_written));
        let before = std::fs::metadata(home.path().join("agents/nadia/soul.md"))
            .unwrap()
            .modified()
            .unwrap();

        let report = run(
            home.path(),
            "nadia",
            ReprovisionOptions {
                tier: Some("default"),
                identity_only: true,
                dry_run: false,
                yes: true,
            },
        )
        .unwrap();

        assert_eq!(read(home.path(), "nadia", "soul.md"), hand_written);
        assert_eq!(
            std::fs::metadata(home.path().join("agents/nadia/soul.md"))
                .unwrap()
                .modified()
                .unwrap(),
            before
        );
        assert!(backups(home.path(), "nadia", "soul.md").is_empty());
        assert!(
            report.contains("--identity-only leaves the two tier axes in disagreement"),
            "the cost must be named at the moment of the gesture: {report}"
        );
    }

    /// **V12** — `--dry-run` writes nothing and still says what it would do.
    #[test]
    fn mika2230_v12_dry_run_writes_nothing() {
        let home = home_with("nadia", Some(CONFIG), Some("name = \"Old\"\n"), None);
        let report = run(
            home.path(),
            "nadia",
            ReprovisionOptions {
                tier: Some("default"),
                identity_only: false,
                dry_run: true,
                yes: true,
            },
        )
        .unwrap();

        assert!(!report.trim().is_empty());
        assert!(report.contains("Dry run"), "{report}");
        assert_eq!(
            read(home.path(), "nadia", "identity.toml"),
            "name = \"Old\"\n"
        );
        assert!(!home.path().join("agents/nadia/soul.md").exists());
        assert!(backups(home.path(), "nadia", "identity.toml").is_empty());
    }

    /// **V13** — targets and backups are owner-only.
    #[cfg(unix)]
    #[test]
    fn mika2230_v13_targets_and_backups_are_0600() {
        use std::os::unix::fs::PermissionsExt;

        let home = home_with(
            "nadia",
            Some(CONFIG),
            Some("name = \"Old\"\n"),
            Some("# old\n"),
        );
        run(home.path(), "nadia", opts(Some("default"))).unwrap();

        let mut paths = vec![
            home.path().join("agents/nadia/identity.toml"),
            home.path().join("agents/nadia/soul.md"),
        ];
        paths.extend(backups(home.path(), "nadia", "identity.toml"));
        paths.extend(backups(home.path(), "nadia", "soul.md"));
        assert_eq!(paths.len(), 4, "two targets and two backups: {paths:?}");

        for path in paths {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{} is {mode:o}", path.display());
        }
    }

    /// **V14** — the negative control of D6. Without it, "does not touch
    /// `config.toml`" is an intention rather than a property — and `config.toml`
    /// is where an operator's hand-picked `llm_provider` lives (mika#2330).
    #[test]
    fn mika2230_v14_config_toml_is_untouched() {
        let config = "llm_provider = \"zai\"\nzai_model = \"glm-5.2\"\n";
        let home = home_with(
            "nadia",
            Some(config),
            Some("name = \"Old\"\n"),
            Some("# old\n"),
        );
        run(home.path(), "nadia", opts(Some("default"))).unwrap();
        assert_eq!(read(home.path(), "nadia", "config.toml"), config);
        assert!(backups(home.path(), "nadia", "config.toml").is_empty());
    }

    // -- Tier (V15–V19) -------------------------------------------------------

    /// **V15/V16** — a complete re-provision to family tier writes both templates,
    /// and the **two** mika#1962 detection axes agree afterwards. That second
    /// assertion is what attests the two-axis correction: an identity-only tool
    /// would pass V15 and fail V16.
    #[test]
    fn mika2230_v15_v16_family_tier_writes_both_axes_and_they_agree() {
        let home = home_with(
            "nadia",
            Some(CONFIG),
            Some(DEFAULT_IDENTITY),
            Some(DEFAULT_SOUL),
        );
        run(home.path(), "nadia", opts(Some("family"))).unwrap();

        assert_eq!(read(home.path(), "nadia", "identity.toml"), FAMILY_IDENTITY);
        assert_eq!(read(home.path(), "nadia", "soul.md"), FAMILY_SOUL);
        assert!(read(home.path(), "nadia", "soul.md").contains(FAMILY_SOUL_MARKER));

        let agent_home = home.path().join("agents/nadia");
        assert!(soul_has_family_marker(&agent_home).unwrap(), "axis 1");
        assert!(
            identity_allowlist_matches_family(&agent_home).unwrap(),
            "axis 2"
        );
    }

    /// **V17** — the cost of `--identity-only`, asserted rather than assumed: the
    /// two axes genuinely diverge. This test exists so the escape hatch's price
    /// is a measured fact, not a paragraph.
    #[test]
    fn mika2230_v17_identity_only_leaves_the_two_axes_diverging() {
        let home = home_with(
            "nadia",
            Some(CONFIG),
            Some(DEFAULT_IDENTITY),
            Some(DEFAULT_SOUL),
        );
        run(
            home.path(),
            "nadia",
            ReprovisionOptions {
                tier: Some("family"),
                identity_only: true,
                dry_run: false,
                yes: true,
            },
        )
        .unwrap();

        let agent_home = home.path().join("agents/nadia");
        assert!(
            !soul_has_family_marker(&agent_home).unwrap(),
            "axis 1 still reads operator"
        );
        assert!(
            identity_allowlist_matches_family(&agent_home).unwrap(),
            "axis 2 now reads family — the axes disagree, which is the stated cost"
        );
    }

    /// **V18** — the flag inherits the tier's own rule instead of inventing one:
    /// an unrecognized value fails closed to the most restricted tools tier
    /// (mika#2023 AC2) and the report names it between quotes.
    #[test]
    fn mika2230_v18_an_unrecognized_tier_value_fails_closed_and_is_named() {
        let home = home_with(
            "nadia",
            Some(CONFIG),
            Some(DEFAULT_IDENTITY),
            Some(DEFAULT_SOUL),
        );
        let report = run(home.path(), "nadia", opts(Some("zorglub"))).unwrap();

        assert!(
            report.contains("\"zorglub\""),
            "the offending value must be named between quotes: {report}"
        );
        assert_eq!(
            read(home.path(), "nadia", "identity.toml"),
            FAMILY_IDENTITY,
            "fail-closed lands on the most restricted tools tier"
        );
    }

    /// **V19** — no `--tier`, no `MIKA_AGENT_TIER`: operator tier, and the report
    /// says the tier came from the default rather than from anywhere else.
    #[test]
    #[serial_test::serial]
    fn mika2230_v19_absent_tier_and_absent_env_resolve_to_default() {
        // Safety: serialized against every other MIKA_AGENT_TIER test.
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
        let home = home_with("nadia", Some(CONFIG), Some("name = \"Old\"\n"), None);
        let report = run(home.path(), "nadia", opts(None)).unwrap();

        assert!(
            report.contains("default (MIKA_AGENT_TIER unset)"),
            "the provenance must be stated: {report}"
        );
        assert_eq!(
            read(home.path(), "nadia", "identity.toml"),
            DEFAULT_IDENTITY
        );
    }

    /// The tier fallback really does read the environment when `--tier` is absent
    /// — the other half of V19, without which "provenance" would be decorative.
    #[test]
    #[serial_test::serial]
    fn mika2230_the_env_is_the_fallback_when_the_flag_is_absent() {
        // Safety: serialized against every other MIKA_AGENT_TIER test.
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "family") };
        let home = home_with("nadia", Some(CONFIG), Some("name = \"Old\"\n"), None);
        let report = run(home.path(), "nadia", opts(None));
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };

        let report = report.unwrap();
        assert!(report.contains("MIKA_AGENT_TIER"), "{report}");
        assert_eq!(read(home.path(), "nadia", "identity.toml"), FAMILY_IDENTITY);
    }

    /// The ungardable direction (family → operator) is warned about **only when
    /// the agent actually reads as family today**, and its negative control is in
    /// the same test.
    ///
    /// A note printed on every operator re-provision is a note operators learn to
    /// skip, and this one has to be read: past this write the markers are gone
    /// and the mika#1962 guard — which detects *family* provisioning only — can
    /// never report the mismatch again.
    #[test]
    fn mika2230_the_ungardable_direction_is_warned_about_only_when_it_applies() {
        let family = home_with(
            "nadia",
            Some(CONFIG),
            Some(FAMILY_IDENTITY),
            Some(FAMILY_SOUL),
        );
        let report = run(
            family.path(),
            "nadia",
            ReprovisionOptions {
                tier: Some("default"),
                identity_only: false,
                dry_run: true,
                yes: true,
            },
        )
        .unwrap();
        assert!(
            report.contains("currently reads as FAMILY-provisioned"),
            "the warning must fire on the population it describes: {report}"
        );

        // Negative control, same flags: an operator agent gets no such warning.
        let operator = home_with(
            "nadia",
            Some(CONFIG),
            Some(DEFAULT_IDENTITY),
            Some(DEFAULT_SOUL),
        );
        let quiet = run(
            operator.path(),
            "nadia",
            ReprovisionOptions {
                tier: Some("default"),
                identity_only: false,
                dry_run: true,
                yes: true,
            },
        )
        .unwrap();
        assert!(
            !quiet.contains("currently reads as FAMILY-provisioned"),
            "boilerplate on the nominal path is how a warning stops being read: {quiet}"
        );
    }

    /// The family-tier write always warns about the SERVICE environment, because
    /// `assert_family_tier_env_consistency` `bail!`s out of `run_server` — a
    /// missing variable there takes every other agent down too.
    #[test]
    fn mika2230_a_family_write_warns_about_the_service_environment() {
        let home = home_with(
            "nadia",
            Some(CONFIG),
            Some(DEFAULT_IDENTITY),
            Some(DEFAULT_SOUL),
        );
        let report = run(
            home.path(),
            "nadia",
            ReprovisionOptions {
                tier: Some("family"),
                identity_only: false,
                dry_run: true,
                yes: true,
            },
        )
        .unwrap();
        assert!(report.contains("SERVICE environment"), "{report}");
        assert!(report.contains("process-wide"), "{report}");
    }

    /// An unreadable present file is refused, never overwritten — the runbook's
    /// § 4 rule, which says the content is still there.
    #[cfg(unix)]
    #[test]
    fn mika2230_an_unreadable_file_is_refused_rather_than_overwritten() {
        use std::os::unix::fs::PermissionsExt;

        // Running as root defeats the permission bits entirely.
        if unsafe { libc_geteuid() } == 0 {
            return;
        }
        let home = home_with("nadia", Some(CONFIG), Some("name = \"Old\"\n"), None);
        let path = home.path().join("agents/nadia/identity.toml");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let err = run(home.path(), "nadia", opts(Some("default"))).unwrap_err();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let msg = err.to_string();
        assert!(msg.contains("cannot be read"), "{msg}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "name = \"Old\"\n");
    }

    #[cfg(unix)]
    unsafe fn libc_geteuid() -> u32 {
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        unsafe { geteuid() }
    }
}
