use anyhow::Result;
use std::io::Read;
use std::sync::Arc;
use uuid::Uuid;

use mika_agent::agent::{self, AgentParams, check_onboarding};
use mika_agent::skills::SkillRegistry;
use mika_agent::tools;

use crate::init;

pub async fn run(message: &str) -> Result<()> {
    let ctx = init::init()?;
    let session_id = Uuid::new_v4().to_string();
    let tool_registry = Arc::new(tools::default_tools());
    let skill_registry = Arc::new(SkillRegistry::from_dir(&ctx.home_dir.join("skills")));

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

    let response = agent::run_agent(&AgentParams {
        db: &ctx.async_db,
        claude: &ctx.claude,
        tools: &tool_registry,
        skills: &skill_registry,
        user_message: &user_message,
        channel_type: "cli",
        session_id: &session_id,
        home_dir: &ctx.home_dir,
        is_onboarding,
        message_sender: None,
        skip_compaction: false,
    })
    .await?;

    println!("{response}");

    // Database shutdown happens automatically via Drop on ctx
    Ok(())
}
