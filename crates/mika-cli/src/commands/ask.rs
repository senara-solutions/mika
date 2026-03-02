use anyhow::Result;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use uuid::Uuid;

use mika_agent::agent::{self, AgentParams, check_onboarding};
use mika_agent::skills::SkillRegistry;
use mika_agent::tools;

use crate::init;

pub async fn run(message: &str, agent_name: &str) -> Result<()> {
    let ctx = init::init_for_agent(agent_name)?;
    let session_id = Uuid::new_v4().to_string();
    let mut tool_registry = tools::default_tools();
    for tool in tools::management_tools_if_needed(&ctx.global_home, &ctx.settings) {
        tool_registry.register(tool);
    }
    let tool_registry = Arc::new(tool_registry);
    let skill_registry = Arc::new(SkillRegistry::from_dir(&ctx.home_dir.join("skills")));
    let http_client = reqwest::Client::new();
    let message_sender =
        crate::init::make_message_sender(&ctx.settings, &ctx.async_db, &http_client);
    let embedding_client = ctx.settings.make_embedding_client();

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

    let is_onboarding = check_onboarding(&ctx.async_db).await;
    let skills_dirty = AtomicBool::new(false);
    let mcp_manager = init::connect_mcp(&ctx.home_dir).await;

    let output = agent::run_agent(&AgentParams {
        db: &ctx.async_db,
        claude: &ctx.claude,
        tools: &tool_registry,
        skills: &skill_registry,
        user_message: &user_message,
        channel_type: "cli",
        session_id: &session_id,
        home_dir: &ctx.home_dir,
        is_onboarding,
        message_sender,
        skip_compaction: false,
        embedding_client: embedding_client.as_ref(),
        thinking: None,
        user_images: &[],
        brave_api_key: ctx.settings.brave_api_key.as_deref(),
        skills_dirty: &skills_dirty,
        mcp_manager: mcp_manager.as_ref(),
        global_home_dir: Some(&ctx.global_home),
    })
    .await?;

    match output.text {
        Some(text) => println!("{text}"),
        None => eprintln!("{}", mika_agent::agent::EMPTY_RESPONSE_FALLBACK),
    }

    // Gracefully shut down MCP server connections
    if let Some(mcp) = mcp_manager {
        mcp.shutdown().await;
    }

    // Database shutdown happens automatically via Drop on ctx
    Ok(())
}
