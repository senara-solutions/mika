use anyhow::Result;
use std::io::Read;
use uuid::Uuid;

use crate::cli::OutputFormat;
use crate::init;

#[derive(serde::Serialize)]
struct AskJsonResponse {
    role: &'static str,
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pending_tasks: Vec<String>,
    /// Runtime metadata envelope. Populated when `--verbose` is set; omitted
    /// otherwise (preserves byte-identical output for existing JSON consumers).
    /// Per-field gating: not every future field needs to be `--verbose`-gated;
    /// the envelope shape supports unconditional fields landing alongside
    /// gated ones without semantics churn.
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<MetadataEnvelope>,
}

/// Runtime metadata for `mika ask` invocations under `--format json`.
///
/// Mirrors the text-mode trailer's role: separates the assistant message
/// (`role`/`content`) from CLI/runtime concerns. Fields here may be
/// individually gated by `--verbose` (e.g., `session_id`) or unconditional
/// (e.g., `task_id` when the CLI flag is provided).
#[derive(Default, serde::Serialize)]
struct MetadataEnvelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    /// Whether the SERVER attested this turn ran with a session-isolated
    /// conversation window (mika#1951). `None` means the server said nothing —
    /// a spirit predating the key, or a path outside synchronous
    /// `message/send` — and is serialized as an **absent** field, never as
    /// `false` and never as the local `--isolated` flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    isolated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<TokensMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_task_id: Option<String>,
}

#[derive(serde::Serialize)]
struct TokensMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_write: Option<u64>,
}

/// Wrap a `send_message_to_agent` failure into an operator-facing error that
/// surfaces the underlying `A2aError` chain (mika#1985).
///
/// The alternate `Display` format (`{err:#}`) walks the anyhow source chain and
/// concatenates each layer with `: ` — so a chain like
/// `A2aError::ClientError` → `anyhow!("connection error: HTTP 500 ...")` becomes
/// visible in the CLI's `eprintln!("Error: {err}")` printer rather than being
/// masked by a single top-level context layer.
///
/// The endpoint URL is preserved so operators can still recognise a wrong-URL
/// misconfiguration; the pre-fix "(is it running?)" hint is dropped because it
/// was actively misleading in the founding incident (spirit *was* running).
///
/// # Why the class is re-attached rather than inherited (mika#2278)
///
/// The flattening above is deliberate and is the whole mika#1985 fix — but it
/// produces a *new* error with no source, so the marker
/// [`mika_cli::remote_ask::TransportClass`] that travelled up from
/// `send_message_to_agent` would be dropped on the floor and a restart would
/// exit `1` like a usage error. Reading the class off the original and
/// re-posing it on the rewritten message keeps both properties: the visible
/// text is byte-identical to the pre-mika#2278 one (which is what
/// `test_wrap_send_error_preserves_underlying_a2a_error_chain` defends) and the
/// exit code still says which of the two happened.
fn wrap_send_error(err: &anyhow::Error, spirit_endpoint: &str) -> anyhow::Error {
    let message = format!("mika ask to {spirit_endpoint} failed: {err:#}");
    if mika_cli::remote_ask::is_transport_failure(err) {
        anyhow::Error::new(mika_cli::remote_ask::TransportClass::new(message))
    } else {
        anyhow::anyhow!("{message}")
    }
}

/// The stderr notice for background work this invocation started (#265).
///
/// Extracted because mika#2270 gave it a second emission site: a turn whose reply
/// was lost still started its background tasks, and the operator needs that fact
/// on the failure path as much as on the success one.
fn pending_callbacks_notice(count: usize) -> String {
    format!(
        "\n[mika] {count} background task(s) started. \
         Open TUI (`mika`) or start server to receive results."
    )
}

/// What to say when `--enable-skill` / `--disable-skill` were passed to an
/// invocation that cannot apply them (mika#1883).
///
/// `None` when neither flag was used — an invocation that asked for nothing is
/// told nothing, and a notice on every turn would bury the one that matters.
///
/// The text names three things, and the third is the one that makes it
/// actionable: the skills concerned, the fact that the turn executes at
/// mika-spirit where these flags do not travel, and **the gesture that does
/// work** — `--only-skill`. A refusal (or a warning) that does not name its own
/// lifting is one that gets worked around by guesswork; that rule was written
/// for gate 2c of mika#2279 and applies here unchanged.
///
/// Expected regime is **zero** of these: the measured usage across `skills/`,
/// `scripts/`, `.claude/` and `crates/` at HEAD `10ad8f8a` is no invocation at
/// all. A line is therefore a caller to migrate — and the count is the
/// measurement that would one day reopen the decision not to build the
/// subtractive half of this channel server-side.
fn inert_skill_flag_notice(enable_skill: &[String], disable_skill: &[String]) -> Option<String> {
    if enable_skill.is_empty() && disable_skill.is_empty() {
        return None;
    }
    let mut named: Vec<String> = Vec::new();
    for name in enable_skill {
        named.push(format!("--enable-skill {name}"));
    }
    for name in disable_skill {
        named.push(format!("--disable-skill {name}"));
    }
    Some(format!(
        "{} had no effect: this turn runs at mika-spirit, and these flags \
         configure only the local skill registry, which is no longer the \
         execution surface (mika#1727). To restrict a turn's skills on the \
         server, use --only-skill <name>; there is no server-side equivalent \
         for forcing a skill on.",
        named.join(", ")
    ))
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    message: &str,
    agent_name: &str,
    task_id: Option<&str>,
    task_complete: bool,
    session_id: Option<&str>,
    parent_task_id: Option<&str>,
    format: &OutputFormat,
    model_override: Option<&str>,
    enable_skill: &[String],
    disable_skill: &[String],
    only_skill: &[String],
    session_isolated: bool,
    verbose: bool,
) -> Result<()> {
    let ctx = init::init_for_agent(agent_name)?;

    // mika#2304: `--model` is no longer spent on `ctx.override_model`. That
    // rebuilt a *local* provider nobody calls on this path any more (mika#1727),
    // while also writing the requested model into `ctx.settings` — which the
    // verbose envelope then read back and printed as if it had served. The flag
    // now travels to the executing surface; the envelope reads the server's
    // attestation.
    //
    // **Arg-level validation stays local and early, but only the part that is
    // provider-independent.** A blank `--model` is refused here, before any round
    // trip. The API-key check moved to the executing side deliberately: this
    // process and the mika-spirit daemon do not share an environment — a service
    // EnvironmentFile against an interactive shell, the exact divergence mika#2293
    // had to make observable for the timeout keys — so a local key check can
    // refuse an override the executing process could serve. The check is not
    // dropped; it moved to the side that can answer it, and its refusal comes
    // back as `remote error: Provider '…' has no API key configured. …`.
    if let Some(model) = model_override
        && model.trim().is_empty()
    {
        anyhow::bail!("--model value must not be empty");
    }

    // Validate task_id format — reject empty or excessively long values
    if let Some(tid) = task_id {
        if tid.is_empty() {
            anyhow::bail!("--task-id value must not be empty");
        }
        if tid.len() > 128 {
            anyhow::bail!("--task-id value too long: {} bytes (max: 128)", tid.len());
        }
    }

    // Use provided session ID or generate a new one.
    // When --session-id is passed (e.g., from claude-asked-relay), messages from the
    // same Claude Code run share a session for grouping and introspection.
    if let Some(s) = session_id
        && s.is_empty()
    {
        anyhow::bail!("--session-id value must not be empty");
    }
    let reusing_session = session_id.is_some();
    // Resolve the canonical session for singleton agents (mika#1401). An explicit
    // --session-id always wins; otherwise a singleton agent (`[session] singleton =
    // true` in identity.toml) reuses its one canonical session and non-singleton
    // agents mint a fresh UUID per invocation. Identity is loaded once here and
    // reused below for the skill allowlist.
    let identity = mika_agent::prompt::load_identity(&ctx.home_dir);
    let canonical_session_id =
        mika_agent::prompt::resolve_canonical_session_id(&identity, ctx.async_db.agent_id());
    let session_id = if let Some(s) = session_id {
        s.to_string()
    } else if let Some(ref canonical) = canonical_session_id {
        canonical.clone()
    } else {
        Uuid::new_v4().to_string()
    };
    let is_canonical_session = canonical_session_id.as_deref() == Some(session_id.as_str());
    // Validate session ownership if reusing an existing session
    if reusing_session
        && let Ok(Some(existing)) = ctx.async_db.get_session(&session_id).await
        && existing.agent_id != ctx.async_db.agent_id()
    {
        anyhow::bail!(
            "Session '{}' belongs to agent '{}', not '{}'",
            session_id,
            existing.agent_id,
            ctx.async_db.agent_id()
        );
    }
    // Create the session. Singleton agents use an idempotent INSERT OR IGNORE on the
    // shared canonical session — task_id correlation rides on the messages
    // (internal-tagged via correlated_task_id), not the shared session row. All other
    // invocations create a per-ask session carrying task_id in metadata and as a
    // first-class column for observability correlation.
    if is_canonical_session {
        if let Err(e) = ctx
            .async_db
            .get_or_create_canonical_session(&session_id, "cli")
            .await
        {
            tracing::warn!(error = %e, "failed to create canonical session");
        }
    } else {
        let session_metadata = task_id.map(|tid| serde_json::json!({"task_id": tid}).to_string());
        if let Err(e) = ctx
            .async_db
            .create_session_with_metadata(
                &session_id,
                ctx.async_db.agent_id(),
                "cli",
                session_metadata.as_deref(),
                task_id,
            )
            .await
        {
            tracing::warn!(error = %e, "failed to create session");
        }
    }
    // Read message from arg, or from stdin if "-"
    let user_message = if message == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf.trim().to_string()
    } else {
        message.to_string()
    };

    if user_message.is_empty() {
        anyhow::bail!("Empty message. Provide a message argument or pipe via stdin with \"-\".");
    }

    // --task-complete path: validate and complete the callback task, then exit.
    // --task-id without --task-complete: correlation only — validate existence, then
    // fall through to the normal agent loop with task_id in session/trace metadata.
    if let Some(tid) = task_id {
        if task_complete {
            // Completion path: size limit + existing completion logic
            const MAX_CALLBACK_RESULT: usize = 100_000; // 100KB, matches server limit
            if user_message.len() > MAX_CALLBACK_RESULT {
                anyhow::bail!(
                    "Callback result too large: {} bytes (max: {MAX_CALLBACK_RESULT})",
                    user_message.len()
                );
            }

            let task = match ctx.async_db.get_task(tid).await {
                Ok(Some(task)) => task,
                Ok(None) => {
                    anyhow::bail!("Task '{}' not found.", tid);
                }
                Err(e) => {
                    anyhow::bail!("Failed to load task '{}': {}", tid, e);
                }
            };

            if task.trigger_type != "callback" {
                anyhow::bail!(
                    "Task '{}' has trigger_type '{}', not 'callback'. \
                     --task-id --task-complete is only for callback tasks.",
                    tid,
                    task.trigger_type
                );
            }
            if !matches!(task.status.as_str(), "pending" | "in_progress") {
                anyhow::bail!(
                    "Task '{}' has status '{}' and cannot be completed.",
                    tid,
                    task.status
                );
            }
            if !ctx
                .async_db
                .update_task_completed(tid, Some(&user_message))
                .await?
            {
                anyhow::bail!(
                    "Task '{}' could not be completed: already in a terminal state.",
                    tid
                );
            }

            // Check if all siblings are done and parent task should be dispatched.
            if let Ok(Some(parent_id)) = ctx.async_db.try_complete_parent_on_sibling_done(tid).await
            {
                tracing::info!(
                    task_id = tid,
                    parent_id = %parent_id,
                    "All sibling tasks complete; parent task ready for dispatch"
                );
            }

            // End the session so the dashboard doesn't show it as "ongoing".
            // No-op for singleton agents — the canonical session is never ended.
            if let Err(e) = ctx
                .async_db
                .end_session_unless_canonical(&session_id, canonical_session_id.as_deref())
                .await
            {
                tracing::warn!(error = %e, "failed to end session");
            }
            return Ok(());
        }

        // Correlation-only path: validate task exists, emit deprecation warning if needed.
        // Uses unscoped lookup because the caller may not own the task.
        // See Database::get_task_unscoped for the ownership-vs-correlation distinction.
        match ctx.async_db.get_task_unscoped(tid).await {
            Ok(Some(task)) => {
                // Deprecation bridge: warn if this looks like an old-style completion call
                if task.trigger_type == "callback"
                    && matches!(task.status.as_str(), "pending" | "in_progress")
                {
                    tracing::warn!(
                        task_id = tid,
                        "DEPRECATED: --task-id without --task-complete no longer completes \
                         callback tasks. Add --task-complete to preserve completion behavior."
                    );
                    eprintln!(
                        "[mika] WARNING: --task-id without --task-complete no longer completes \
                         callback tasks. Add --task-complete to preserve completion behavior."
                    );
                }
            }
            Ok(None) => {
                anyhow::bail!("Task '{}' not found.", tid);
            }
            Err(e) => {
                tracing::warn!(error = %e, task_id = tid, "failed to validate task existence");
            }
        }
        // Fall through to normal agent loop with task_id in session metadata + trace
    }

    // Prepend task context if --parent-task-id is provided
    let user_message = if let Some(pt) = parent_task_id {
        format!("[work-item:{pt}] {user_message}")
    } else {
        user_message
    };

    // --- #1727 TUI/CLI thin-client slice ---
    // The one-shot agent loop no longer runs in-process here. It is delegated to
    // the local mika-spirit daemon over A2A (`message/send`), mirroring the
    // `--remote` cloud path in `remote_ask.rs`. mika-spirit owns skills, tools,
    // MCP, and per-run token accounting; this process is now a thin client that
    // ships the prompt and renders the returned Task.
    //
    // mika#1883 closed the last two deferrals this block used to list, and the
    // list is gone with them: a follow-up note must not outlive its own
    // resolution (the rule mika#2070 applied to the third one).
    //
    // Resolved (mika#1883), per-run token usage: the `Task` now carries the
    // turn's **total** under `mika.run_usage`, so `--verbose` reports `tokens.*`
    // again. The obvious wiring — forwarding `AgentOutput.usage` — was refused
    // because that field is the *last* call's (the loop overwrites it at every
    // step), so a twenty-step turn would have reported one call in twenty-one
    // under the label "per-run usage". The aggregate is summed in the loop
    // instead, continuation included.
    //
    // Settled (mika#1883), `--enable-skill` / `--disable-skill`: they are inert
    // and stay inert, and the invocation now **says so** on stderr rather than
    // looking accepted. The additive half of the config channel keeps the refusal
    // below; the subtractive half is not built for `--disable-skill` either,
    // because `--only-skill` already covers the measured need and a third
    // selection semantic on one turn is a composition nobody wants to debug —
    // the reason `cli.rs` already states on `--only-skill` itself.
    //
    // Resolved (mika#2304): `--model` used to sit in that list, and its
    // presence there was measured as a false green rather than a missing feature.
    // It reached the *local* `Settings`, which the verbose envelope below then
    // read and printed — so `--verbose` asserted the override with full authority
    // while the turn ran under the agent's configured model. It now travels to
    // spirit under `mika.model_override`, and the envelope reports the model the
    // server attests instead of anything this process knows.
    //
    // Partially resolved (mika#2363): `--only-skill` DOES reach spirit, through
    // `message/send` request metadata. It is deliberately only the **subtractive**
    // half of that missing config channel — it can evict a skill from the turn,
    // never activate one. The additive half (`--enable-skill` →
    // `apply_transient_always_on` server-side) stays deferred with its reason:
    // it would let any authenticated caller of `/a2a/{agent}` force one of the
    // agent's skills to `always_on`, and it buys nothing for the case that
    // motivated the channel — the skill `_arch_ask` needs is already `always_on`.
    //
    // Resolved (mika#2070): the local bookkeeping session is no longer orphaned.
    // Its id travels to spirit in `message/send` request metadata, and spirit runs
    // the turn under it whenever it already owns that session row — so the agent
    // turns and the `turn_usage` events land on the session this process created.
    // An id spirit cannot own is refused silently and it mints `a2a-<task_id>` as
    // before.

    // Preserve arg-level validation: --enable-skill / --disable-skill must not
    // conflict on the same skill name.
    for enable_name in enable_skill {
        if disable_skill
            .iter()
            .any(|d| d.eq_ignore_ascii_case(enable_name))
        {
            anyhow::bail!(
                "Cannot both enable and disable skill '{enable_name}' in the same invocation"
            );
        }
    }

    // mika#1883 — say the no-op out loud. The two flags above configure the
    // *local* registry, which since mika#1727 is not where the turn runs, so
    // they are validated and then abandoned in transit. Until now that happened
    // in silence: the invocation looked accepted and the turn ran unrestricted,
    // which is the shape of false green this ticket's sibling field exists to
    // remove one measurement over.
    //
    // A warning and not a refusal, and the asymmetry is measured: an unapplied
    // skill restriction makes a turn *wider*, which is visible and falsifies no
    // measurement — the same reason `mika.only_skills` is fail-soft where
    // `mika.model_override` is fail-closed.
    if let Some(notice) = inert_skill_flag_notice(enable_skill, disable_skill) {
        tracing::warn!(event = "cli_skill_flag_inert", "{notice}");
    }

    let started = std::time::Instant::now();

    // Dispatch to the local mika-spirit A2A endpoint: {spirit_url}/a2a/{agent_name}.
    let spirit_endpoint = format!(
        "{}/a2a/{}",
        crate::commands::dashboard::spirit_url(),
        agent_name
    );
    // mika#1985: route the failure through `wrap_send_error` so the underlying
    // A2aError message reaches stdout. The old `.with_context(...)` added a new
    // top context; `{err}` in the outer CLI printer prints only the top layer,
    // hiding HTTP status, reqwest transport errors, A2A state transitions, and
    // server-side LLM/OAuth errors. The 2026-08-24 Prime OAuth incident cost ~2h
    // of duo diag under this masking. See `wrap_send_error` for the shape and
    // the test `test_wrap_send_error_preserves_underlying_a2a_error_chain`.
    // mika#2070: carry this invocation's session id to spirit. Spirit adopts it as
    // the agent-loop session when it already owns the row, so `turn_usage` is
    // emitted under the caller's session instead of spirit's `a2a-<task_id>`.
    //
    // Two limits worth knowing before using this to attribute cost:
    //   * The id we send is whatever this invocation resolved — an explicit
    //     `--session-id`, else a singleton agent's canonical session, else a
    //     fresh UUID. Callers that share a session id deliberately (a singleton
    //     agent, a retry that wants the agent to see its own prior turn) share
    //     their `turn_usage` too. Per-run attribution means a per-run
    //     `--session-id`.
    //   * A turn that delegates (`delegate_task`) or starts a team run spends
    //     tokens under `delegate-*` / `team-*` sessions of its own. Filtering on
    //     this id captures the turn, not the work it fanned out.
    let task = mika_cli::remote_ask::send_message_to_agent(
        &user_message,
        &spirit_endpoint,
        Some(session_id.as_str()),
        only_skill,
        model_override,
        session_isolated,
    )
    .await
    .map_err(|e| wrap_send_error(&e, &spirit_endpoint))?;

    // End the local bookkeeping session regardless of outcome so the dashboard
    // shows duration. No-op for singleton agents — the canonical session is never
    // ended (mika#1401).
    if let Err(e) = ctx
        .async_db
        .end_session_unless_canonical(&session_id, canonical_session_id.as_deref())
        .await
    {
        tracing::warn!(error = %e, "failed to end session");
    }

    // Check for pending callback tasks spawned during the agent loop (#265).
    // In `mika ask` there is no TaskEngine to poll for callbacks — the user needs
    // TUI or server to receive results from long-running background tasks.
    let pending_callbacks = match ctx
        .async_db
        .get_pending_callbacks_for_session(&session_id)
        .await
    {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(error = %e, "failed to check pending callbacks");
            vec![]
        }
    };

    // Extract the assistant text from the returned Task (mika#2270). A Task the
    // renderer cannot read is a LOST turn, not an agent with nothing to say: the
    // engine produced its answer, paid for it, and the return channel dropped it.
    // So this fails naming what it inspected instead of mapping to `None`, which
    // is what let eleven consecutive calls exit 0 with `.content` absent while the
    // verdict sat in the server log.
    //
    // The pending-callback notice is emitted before returning: background work was
    // genuinely started, and the failure of the reply channel must not also cost
    // the operator that fact.
    let content = match mika_cli::remote_ask::render_task_parts(&task) {
        Ok(text) => text,
        Err(empty) => {
            if !pending_callbacks.is_empty() {
                eprintln!("{}", pending_callbacks_notice(pending_callbacks.len()));
            }
            return Err(anyhow::Error::new(empty));
        }
    };

    // Build the metadata envelope. Verbose-gated fields are populated only
    // when `--verbose`; unconditional fields (`task_id`, `parent_task_id`)
    // are populated whenever their CLI flag was provided.
    let elapsed_ms = started.elapsed().as_millis() as u64;
    // mika#2304 D3: the model comes from the server's attestation on the returned
    // Task, never from `ctx.settings`. Reading the local settings is what made
    // `--verbose` assert an override that had not happened — the field was not
    // absent or null, it stated the requested model with authority while the turn
    // ran under another. `None` here means the server did not attest (a binary
    // older than mika#2304, or a path outside synchronous `message/send`), and
    // the only honest rendering of that is *no model at all*.
    let model_string = mika_cli::remote_ask::attested_model(&task).map(str::to_string);
    // mika#1951, same rule and same reason one field down: the isolation shown is
    // the one the server attested on the returned Task. Reading `session_isolated`
    // back here would print the flag this process was handed — an assertion about
    // a turn it did not run, and the precise shape of the false green mika#2304
    // measured on the `model:` line.
    let isolated_attested = mika_cli::remote_ask::attested_session_isolation(&task);
    // mika#1883, same rule and same reader as the two fields above: the token
    // counts shown are the ones the server attested for the whole turn, read
    // through `mika-a2a`'s single decoder so `--remote` and this path cannot
    // answer the same question differently. `None` means the server attested
    // nothing (a spirit older than mika#1883, or a turn that made no call whose
    // usage could be read), and the honest rendering of that is no `tokens.*`
    // line at all — never a zero, which is indistinguishable from a real turn.
    let tokens_attested = mika_cli::remote_ask::attested_run_usage(&task).map(|u| TokensMetadata {
        input: Some(u.input),
        output: Some(u.output),
        cache_read: u.cache_read,
        cache_write: u.cache_write,
    });

    let envelope = MetadataEnvelope {
        // Unconditional fields — present whenever the CLI flag was provided
        task_id: task_id.map(|s| s.to_string()),
        parent_task_id: parent_task_id.map(|s| s.to_string()),
        // Verbose-gated fields
        session_id: if verbose {
            Some(session_id.clone())
        } else {
            None
        },
        model: if verbose { model_string } else { None },
        isolated: if verbose { isolated_attested } else { None },
        agent_id: if verbose {
            Some(agent_name.to_string())
        } else {
            None
        },
        latency_ms: if verbose { Some(elapsed_ms) } else { None },
        // mika#1883: the `Task` now carries the turn's total under
        // `mika.run_usage`, so verbose `tokens.*` reports a measured sum again
        // — of every call of this turn, continuation included, and never of the
        // last one alone.
        tokens: if verbose { tokens_attested } else { None },
    };

    // Emit None when all fields are absent so the top-level `metadata` key
    // is omitted from JSON (preserves byte-identical output for non-verbose,
    // non-task invocations).
    let has_any_field = envelope.session_id.is_some()
        || envelope.model.is_some()
        || envelope.isolated.is_some()
        || envelope.agent_id.is_some()
        || envelope.latency_ms.is_some()
        || envelope.tokens.is_some()
        || envelope.task_id.is_some()
        || envelope.parent_task_id.is_some();
    let metadata = if has_any_field { Some(envelope) } else { None };

    match format {
        OutputFormat::Text => {
            println!("{content}");
            if !pending_callbacks.is_empty() {
                eprintln!("{}", pending_callbacks_notice(pending_callbacks.len()));
            }
            // Text-mode trailer: one key: value per populated field.
            // Blank line separates response body from metadata trailer.
            if let Some(ref meta) = metadata {
                println!();
                if let Some(ref v) = meta.session_id {
                    println!("session_id: {v}");
                }
                // mika#2304: under `--verbose` the line is always emitted, and
                // says so when the server attested nothing. Silently omitting it
                // would leave the operator unable to tell "no attestation" from
                // "the trailer changed shape", and it is precisely the operator
                // who is running a pre-flight and needs to know which of the two
                // they are looking at.
                match (&meta.model, verbose) {
                    (Some(v), _) => println!("model: {v}"),
                    (None, true) => println!("{}", mika_cli::remote_ask::NO_ATTESTATION_LINE),
                    (None, false) => {}
                }
                // mika#1951: same three-arm shape as `model:` above, for the same
                // reason. Under `--verbose` the line is always emitted and says
                // so when the server attested nothing — an operator running a
                // bench must be able to tell "not isolated" from "this server
                // does not know how to tell me", and silence conflates them.
                match (meta.isolated, verbose) {
                    (Some(v), _) => println!("isolated: {v}"),
                    (None, true) => {
                        println!("{}", mika_cli::remote_ask::NO_ISOLATION_ATTESTATION_LINE)
                    }
                    (None, false) => {}
                }
                if let Some(ref v) = meta.agent_id {
                    println!("agent_id: {v}");
                }
                if let Some(v) = meta.latency_ms {
                    println!("latency_ms: {v}");
                }
                if let Some(ref t) = meta.tokens {
                    if let Some(v) = t.input {
                        println!("tokens.input: {v}");
                    }
                    if let Some(v) = t.output {
                        println!("tokens.output: {v}");
                    }
                    if let Some(v) = t.cache_read {
                        println!("tokens.cache_read: {v}");
                    }
                    if let Some(v) = t.cache_write {
                        println!("tokens.cache_write: {v}");
                    }
                }
                if let Some(ref v) = meta.task_id {
                    println!("task_id: {v}");
                }
                if let Some(ref v) = meta.parent_task_id {
                    println!("parent_task_id: {v}");
                }
            }
        }
        OutputFormat::Json => {
            let response = AskJsonResponse {
                role: "assistant",
                // `Option` is kept on the envelope for the team path and for
                // consumers that already tolerate a null; on this path it is now
                // always `Some` — mika#2270 turned an absent `.content` from a
                // possible answer into an error.
                content: Some(content),
                task_id: task_id.map(|s| s.to_string()),
                pending_tasks: pending_callbacks,
                metadata,
            };
            println!("{}", serde_json::to_string(&response)?);
        }
        OutputFormat::Yaml => {
            let response = AskJsonResponse {
                role: "assistant",
                content: Some(content),
                task_id: task_id.map(|s| s.to_string()),
                pending_tasks: pending_callbacks,
                metadata,
            };
            print!("{}", serde_yaml::to_string(&response)?);
        }
    }

    // Database shutdown happens automatically via Drop on ctx
    Ok(())
}

/// Extended JSON response for team runs.
#[derive(serde::Serialize)]
struct AskTeamJsonResponse {
    role: &'static str,
    content: Option<String>,
    team_run: TeamRunMeta,
}

#[derive(serde::Serialize)]
struct TeamRunMeta {
    run_id: String,
    status: String,
    iterations: u32,
}

/// Run a team workflow in non-interactive mode (mika ask --team).
///
/// Runs the full team cycle (decompose → execute → review → deliver),
/// prints progress to stderr and the deliverable to stdout.
pub async fn run_team_ask(
    team_name: &str,
    message: &str,
    run_id: Option<&str>,
    format: &OutputFormat,
    global_home: &std::path::Path,
) -> Result<()> {
    use mika_agent::teams::types::{RunStatus, TeamEvent};
    use mika_common::config::Settings;

    // Read message from stdin if "-"
    let goal = if message == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf.trim().to_string()
    } else {
        message.to_string()
    };

    if goal.is_empty() {
        anyhow::bail!("Empty message. Provide a goal for the team.");
    }

    // Validate --run-id format before any filesystem/DB use (defense-in-depth)
    if let Some(ref_id) = run_id {
        if uuid::Uuid::parse_str(ref_id).is_err() {
            anyhow::bail!(
                "Invalid --run-id format. Expected a UUID (e.g., from a previous team run)."
            );
        }
        let db_path = mika_common::home::container_db_path(global_home);
        if db_path.exists() {
            let db = mika_agent::db::Database::open(&db_path)?;
            match db.load_team_run_by_id(ref_id) {
                Ok(Some(run)) => {
                    if run.team_name != team_name {
                        anyhow::bail!(
                            "Run '{}' belongs to team '{}', not '{}'.",
                            ref_id,
                            run.team_name,
                            team_name
                        );
                    }
                    if run.status == "running" {
                        anyhow::bail!(
                            "Run '{}' is still running. Cannot reference a running run.",
                            ref_id
                        );
                    }
                }
                Ok(None) => {
                    anyhow::bail!("Run '{}' not found.", ref_id);
                }
                Err(e) => {
                    anyhow::bail!("Failed to look up run '{}': {}", ref_id, e);
                }
            }
        } else {
            anyhow::bail!("No database found. Run `mika` first to initialize.");
        }
    }

    let settings = Settings::load(global_home)?;

    let callback = |event: TeamEvent| match event {
        TeamEvent::Progress(msg) => {
            eprintln!("  > {msg}");
        }
        TeamEvent::PhaseChanged { phase, iteration } => {
            eprintln!("  > Phase: {phase} (iteration {iteration})");
        }
        TeamEvent::AgentStarted { agent, role } => {
            eprintln!("  > Agent {agent} ({role}) started");
        }
        TeamEvent::AgentCompleted { agent, .. } => eprintln!("  > {agent} completed"),
        TeamEvent::AgentFailed { agent, error } => {
            eprintln!("  > {agent} failed: {error}");
        }
        TeamEvent::TasksAssigned { tasks, iteration } => {
            let names: Vec<_> = tasks.iter().map(|t| t.agent.as_str()).collect();
            eprintln!(
                "  > Iteration {iteration}: assigned tasks to {}",
                names.join(", ")
            );
        }
        TeamEvent::CriticReview {
            approved,
            feedback,
            iteration,
        } => {
            let verdict = if approved { "approved" } else { "rejected" };
            eprintln!("  > Critic (iteration {iteration}): {verdict}. {feedback}");
        }
        TeamEvent::Deliverable(_) => {} // handled below
        TeamEvent::RunFailed(_) => {}   // handled below
    };

    let team_db = crate::commands::teams::open_container_db_async(global_home)?;

    let github_app = mika_common::github_app::GitHubApp::from_settings(&settings);
    let run = mika_agent::teams::run_team(
        team_name,
        &goal,
        global_home,
        &settings,
        Some(Box::new(callback)),
        team_db.clone(),
        run_id,
        github_app,
        None, // CLI: no AppState for session-scoped dedup (#821)
        // mika#1962 — CLI process start is the tier init boundary.
        mika_common::home::AgentTier::from_env(),
        // mika#2290 — same boundary for the posed hosting fact.
        mika_common::home::Deployment::from_env(),
    )
    .await?;
    team_db.shutdown();

    let is_failure = matches!(&run.status, RunStatus::Failed(_));

    match format {
        OutputFormat::Text => {
            if let Some(ref deliverable) = run.deliverable {
                println!("{deliverable}");
            } else if let RunStatus::Failed(ref msg) = run.status {
                eprintln!("Error: {msg}");
            }
        }
        OutputFormat::Json => {
            let response = AskTeamJsonResponse {
                role: "assistant",
                content: run.deliverable.clone(),
                team_run: TeamRunMeta {
                    run_id: run.run_id.clone(),
                    status: format!("{}", run.status),
                    iterations: run.iteration,
                },
            };
            println!("{}", serde_json::to_string(&response)?);
        }
        OutputFormat::Yaml => {
            let response = AskTeamJsonResponse {
                role: "assistant",
                content: run.deliverable.clone(),
                team_run: TeamRunMeta {
                    run_id: run.run_id.clone(),
                    status: format!("{}", run.status),
                    iterations: run.iteration,
                },
            };
            print!("{}", serde_yaml::to_string(&response)?);
        }
    }

    if is_failure {
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_response_with_content() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Hello, world!".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, r#"{"role":"assistant","content":"Hello, world!"}"#);
    }

    #[test]
    fn test_json_response_with_null_content() {
        let response = AskJsonResponse {
            role: "assistant",
            content: None,
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, r#"{"role":"assistant","content":null}"#);
    }

    /// mika#1985 regression: the CLI wrapper around `send_message_to_agent` must
    /// surface the underlying `A2aError` message text in the visible error, not just
    /// the generic "failed to reach mika-spirit" wrapper.
    ///
    /// The founding incident (2026-08-24 Prime OAuth loss) hid a server-side
    /// `LLM provider error: OAuth token resolution failed` under a bare
    /// "failed to reach mika-spirit... is it running?" for ~2h of duo diag. This
    /// test locks in the mask-through fix: the visible message must contain
    /// (a) the underlying reason string and (b) the endpoint URL (actionable
    /// context). Since mika#2036 that reason is a full sentence naming the
    /// transport failure *and* what became of the work; the mask-through
    /// contract this test defends is unchanged.
    ///
    /// The test exercises the real `wrap_send_error` helper (the production
    /// wrapper used at commands/ask.rs:~320) against a synthetic anyhow error
    /// shaped like what `send_message_to_agent` returns on `A2aError::ClientError`
    /// — no real A2A server, no reqwest — so failure of this test proves the
    /// production wrapper regressed, independent of any live-endpoint state.
    #[test]
    fn test_wrap_send_error_preserves_underlying_a2a_error_chain() {
        // Shape mirrors `send_message_to_agent`'s `A2aError::ClientError` arm,
        // which since mika#2036 bails with `remote_ask::transport_error_message`.
        let underlying: anyhow::Error = anyhow::anyhow!(
            "HTTP 500 from http://test.local/a2a/test-agent; \
             the server holds no task for context ctx-1 — the request did not land"
        );
        let spirit_endpoint = "http://test.local/a2a/test-agent";

        // Exercise the real production helper — a regression that reverts to
        // `.with_context()` (or any wrapper using `{e}` instead of `{e:#}`) will
        // make this test fail.
        let wrapped: anyhow::Error = wrap_send_error(&underlying, spirit_endpoint);

        let visible = format!("{wrapped}");

        // (a) Mask-through: the underlying reason must appear in the visible message.
        assert!(
            visible.contains("holds no task"),
            "visible error must surface underlying A2aError reason; got: {visible}"
        );
        assert!(
            visible.contains("HTTP 500"),
            "visible error must surface underlying HTTP status; got: {visible}"
        );

        // (b) Endpoint context preserved: the wrapper still names the URL for
        // actionable "wrong endpoint" diagnosis.
        assert!(
            visible.contains(spirit_endpoint),
            "visible error must surface endpoint URL; got: {visible}"
        );

        // (c) Anti-regression: the misleading pre-fix wrapper text must NOT appear.
        // If a future change re-introduces `.with_context(|| "failed to reach mika-spirit... (is it running?)")`
        // the substring below will show up in `visible` and this assertion will fail.
        assert!(
            !visible.contains("is it running?"),
            "the misleading pre-fix 'is it running?' wrapper must not resurface; got: {visible}"
        );
    }

    #[test]
    fn test_json_response_with_special_characters() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Line 1\nLine 2\t\"quoted\"".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["role"], "assistant");
        assert_eq!(parsed["content"], "Line 1\nLine 2\t\"quoted\"");
    }

    #[test]
    fn test_json_response_with_pending_tasks() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("I've started the implementation.".to_string()),
            task_id: None,
            pending_tasks: vec!["task-abc-123".to_string(), "task-def-456".to_string()],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["role"], "assistant");
        assert!(parsed["pending_tasks"].is_array());
        assert_eq!(parsed["pending_tasks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_json_response_omits_empty_pending_tasks() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Done.".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        // pending_tasks should be omitted entirely when empty
        assert!(!json.contains("pending_tasks"));
    }

    #[test]
    fn test_json_response_with_task_id() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Permission granted.".to_string()),
            task_id: Some("abc-123-def".to_string()),
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["role"], "assistant");
        assert_eq!(parsed["task_id"], "abc-123-def");
        // pending_tasks should be omitted when empty
        assert!(parsed.get("pending_tasks").is_none());
    }

    #[test]
    fn test_verbose_trailer_format() {
        // The verbose trailer must be a standalone `session_id: <uuid>` line
        // that downstream parsers can match by key name.
        let session_id = uuid::Uuid::new_v4().to_string();
        let trailer = format!("session_id: {session_id}");

        // Must start with the key name
        assert!(trailer.starts_with("session_id: "));

        // The value after "session_id: " must be a valid UUID
        let value = trailer.strip_prefix("session_id: ").unwrap();
        assert!(uuid::Uuid::parse_str(value).is_ok());
    }

    #[test]
    fn test_json_response_omits_none_task_id() {
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Hello.".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        // task_id should be omitted when None
        assert!(!json.contains("task_id"));
    }

    #[test]
    fn test_json_response_omits_metadata_when_none() {
        // Existing JSON consumers (no --verbose) must see byte-identical
        // output to pre-#829: the `metadata` key is skipped entirely when
        // None, not emitted as `"metadata":null`.
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Hello, world!".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, r#"{"role":"assistant","content":"Hello, world!"}"#);
        assert!(!json.contains("metadata"));
    }

    #[test]
    fn test_json_response_includes_metadata_session_id_when_verbose() {
        // When --verbose is set, the JSON envelope must carry session_id
        // inside a nested `metadata` object — separating runtime metadata
        // from the assistant message shape (mirrors the text-mode trailer's
        // conceptual separation).
        let session_id = uuid::Uuid::new_v4().to_string();
        let response = AskJsonResponse {
            role: "assistant",
            content: Some("Here.".to_string()),
            task_id: None,
            pending_tasks: vec![],
            metadata: Some(MetadataEnvelope {
                session_id: Some(session_id.clone()),
                ..Default::default()
            }),
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Metadata must be a nested object, not a top-level field.
        assert!(parsed["metadata"].is_object());
        assert_eq!(parsed["metadata"]["session_id"], session_id);

        // The session_id value must round-trip as a valid UUID.
        let value = parsed["metadata"]["session_id"].as_str().unwrap();
        assert!(uuid::Uuid::parse_str(value).is_ok());

        // session_id must NOT appear at the top level — that would entangle
        // the message shape with runtime metadata.
        assert!(parsed.get("session_id").is_none());
    }

    #[test]
    fn test_metadata_envelope_default_serializes_empty() {
        // Default envelope with all fields None serializes to `{}`.
        let envelope = MetadataEnvelope::default();
        let json = serde_json::to_string(&envelope).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn test_metadata_envelope_partial_population() {
        // Only session_id set — other fields absent from JSON.
        let envelope = MetadataEnvelope {
            session_id: Some("abc-123".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["session_id"], "abc-123");
        assert!(parsed.get("model").is_none());
        assert!(parsed.get("agent_id").is_none());
        assert!(parsed.get("latency_ms").is_none());
        assert!(parsed.get("tokens").is_none());
        assert!(parsed.get("task_id").is_none());
        assert!(parsed.get("parent_task_id").is_none());
    }

    #[test]
    fn test_metadata_envelope_full_population() {
        // All fields populated — complete JSON shape.
        let envelope = MetadataEnvelope {
            session_id: Some("sess-1".to_string()),
            model: Some("anthropic/claude-sonnet-4-6".to_string()),
            isolated: Some(true),
            agent_id: Some("mika-dev".to_string()),
            latency_ms: Some(1234),
            tokens: Some(TokensMetadata {
                input: Some(100),
                output: Some(50),
                cache_read: Some(80),
                cache_write: Some(20),
            }),
            task_id: Some("task-1".to_string()),
            parent_task_id: Some("parent-1".to_string()),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["session_id"], "sess-1");
        assert_eq!(parsed["model"], "anthropic/claude-sonnet-4-6");
        assert_eq!(parsed["agent_id"], "mika-dev");
        assert_eq!(parsed["latency_ms"], 1234);
        assert_eq!(parsed["tokens"]["input"], 100);
        assert_eq!(parsed["tokens"]["output"], 50);
        assert_eq!(parsed["tokens"]["cache_read"], 80);
        assert_eq!(parsed["tokens"]["cache_write"], 20);
        assert_eq!(parsed["task_id"], "task-1");
        assert_eq!(parsed["parent_task_id"], "parent-1");
        assert_eq!(parsed["isolated"], serde_json::Value::Bool(true));
    }

    #[test]
    fn test_metadata_envelope_verbose_without_usage() {
        // Verbose with no LLM usage — tokens field absent.
        let envelope = MetadataEnvelope {
            session_id: Some("sess-2".to_string()),
            model: Some("openai/gpt-4o".to_string()),
            agent_id: Some("mika".to_string()),
            latency_ms: Some(500),
            tokens: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["model"], "openai/gpt-4o");
        assert_eq!(parsed["latency_ms"], 500);
        assert!(parsed.get("tokens").is_none());
    }

    #[test]
    fn test_metadata_envelope_unconditional_task_id_only() {
        // Non-verbose with --task-id: envelope has only task_id.
        let envelope = MetadataEnvelope {
            task_id: Some("abc".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["task_id"], "abc");
        // No verbose-gated fields
        assert!(parsed.get("session_id").is_none());
        assert!(parsed.get("model").is_none());
        assert!(parsed.get("isolated").is_none());
        assert!(parsed.get("agent_id").is_none());
        assert!(parsed.get("latency_ms").is_none());
        assert!(parsed.get("tokens").is_none());
    }

    // --- mika#1951 U3: the envelope reports the server, never the flag --------

    /// **The attestation is a bool, and `false` is a value — not an absence.**
    ///
    /// `skip_serializing_if = "Option::is_none"` on an `Option<bool>` is what
    /// keeps the three states apart on the wire: `true` (the server isolated the
    /// turn), `false` (the server understood and did not), and the key being
    /// **absent** (the server said nothing at all). A plain `bool` would have
    /// collapsed the last two into `false`, which reads as "not isolated" and is
    /// the exact conflation that lets a bench against an old spirit look
    /// measured.
    #[test]
    fn mika1951_the_isolation_attestation_keeps_false_and_absent_apart() {
        let attested_false = MetadataEnvelope {
            isolated: Some(false),
            ..Default::default()
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&attested_false).unwrap()).unwrap();
        assert_eq!(
            parsed["isolated"],
            serde_json::Value::Bool(false),
            "an attested `false` must be emitted, not skipped: {parsed}"
        );

        let unattested = MetadataEnvelope::default();
        let parsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&unattested).unwrap()).unwrap();
        assert!(
            parsed.get("isolated").is_none(),
            "an unattested turn must omit the key, never emit `false`: {parsed}"
        );
    }

    /// **The envelope is fed by the Task, and nothing else reads the flag.**
    ///
    /// `run` is handed `session_isolated` and posts it on the wire; the only
    /// other thing it may do with it is nothing. Reading it back into the
    /// envelope would print an isolation this process asked for rather than one
    /// that happened — the mika#2304 false green, one field over. No behavioural
    /// test can see that substitution: both renderings say `isolated: true` on
    /// the happy path and differ only against a server that ignored the key,
    /// which is the population a unit test has no server for.
    #[test]
    fn mika1951_the_envelope_never_reads_the_local_flag() {
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let source = scanner.production_of(&scanner.src_root().join("commands/ask.rs"));

        // Two mentions and two only: the parameter's declaration, and the one
        // place it is handed to `send_message_to_agent`. Comment lines are
        // excluded — the prose above the envelope names the flag precisely in
        // order to say it is *not* read, and a guard that counted that mention
        // would forbid explaining itself.
        let mentions = source
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .filter(|l| l.contains("session_isolated"))
            .count();
        assert_eq!(
            mentions, 2,
            "expected `session_isolated` to appear exactly twice (declared, then \
             passed to the wire), found {mentions} — a third mention is the flag \
             being read back, which would make `--verbose` answer for the server"
        );
        assert!(
            source.contains("attested_session_isolation(&task)"),
            "the envelope must be fed by the server's attestation on the Task"
        );
    }

    /// **AC6** — the two inert flags are said out loud, and never on stdout.
    ///
    /// Three properties in one test because they are one contract: the notice
    /// exists only when a flag was passed, it names the working gesture, and it
    /// leaves stdout alone.
    ///
    /// The stdout half is asserted **structurally**, on this file's own source.
    /// A behavioural test cannot see it: moving the notice from `warn!` to
    /// `println!` would make every assertion about its *content* stay green
    /// while `mika ask --format json` stopped being parseable — the silent class
    /// this repo answers with a scan wherever it meets it. The predicate is on
    /// the notice's own identifier, which is the only thing the regression can
    /// hand to a stdout writer.
    #[test]
    fn mika1883_the_inert_skill_flags_warn_on_stderr_only() {
        // Nothing passed, nothing said: a notice on every invocation would bury
        // the one that matters.
        assert!(inert_skill_flag_notice(&[], &[]).is_none());

        let notice = inert_skill_flag_notice(
            &["qa-review".to_string()],
            &["self-dev".to_string(), "dev-pilot".to_string()],
        )
        .expect("a flag was passed, so the inertia must be stated");

        for named in [
            "--enable-skill qa-review",
            "--disable-skill self-dev",
            "--disable-skill dev-pilot",
        ] {
            assert!(notice.contains(named), "{named} unnamed in: {notice}");
        }
        assert!(
            notice.contains("mika-spirit"),
            "the notice must say WHERE the turn runs, or the operator cannot act \
             on it: {notice}"
        );
        assert!(
            notice.contains("--only-skill"),
            "a refusal that does not name its own lifting is one that gets worked \
             around by guesswork (mika#2279 gate 2c): {notice}"
        );

        // Either flag alone is enough — the two are independent.
        assert!(inert_skill_flag_notice(&["x".to_string()], &[]).is_some());
        assert!(inert_skill_flag_notice(&[], &["y".to_string()]).is_some());

        // Structural half: the notice never reaches a stdout writer.
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let this_file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("commands")
            .join("ask.rs");
        let production = scanner.production_of(&this_file);
        // Assembled at runtime so this guard's own source is not its first
        // offender (mika#2220's rule).
        let stdout_writers = ["print".to_string() + "ln!", "print".to_string() + "!"];
        for (i, line) in production.lines().enumerate() {
            if !line.contains("inert_skill_flag_notice") {
                continue;
            }
            for writer in &stdout_writers {
                assert!(
                    !line.contains(writer.as_str()),
                    "mika#1883 — the inert-flag notice reaches stdout at ask.rs:{}: {}\n\
                     It must stay on `tracing::warn!`: `--format json` is parsed by \
                     `_arch_ask` and every scripted caller, and one extra stdout line \
                     breaks a wire contract for a message of convenience.",
                    i + 1,
                    line.trim()
                );
            }
        }
    }

    #[test]
    fn test_tokens_metadata_partial_cache() {
        // Only input/output tokens, no cache fields.
        let tokens = TokensMetadata {
            input: Some(200),
            output: Some(100),
            cache_read: None,
            cache_write: None,
        };
        let json = serde_json::to_string(&tokens).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["input"], 200);
        assert_eq!(parsed["output"], 100);
        assert!(parsed.get("cache_read").is_none());
        assert!(parsed.get("cache_write").is_none());
    }
}
