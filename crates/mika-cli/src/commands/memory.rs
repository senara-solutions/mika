use anyhow::Result;
use mika_agent::db::{CORE_MEMORY_SECTIONS, core_memory_section_names};

use crate::cli::{MemoryArgs, MemoryCommand};
use crate::init;

pub async fn run(args: MemoryArgs, agent_name: &str) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;
    let db = &ctx.async_db;

    match args.command {
        None => {
            // Show all core memory blocks
            let entries = db.get_all_core_memory().await?;
            if entries.is_empty() {
                println!("\nNo core memory entries.\n");
            } else {
                println!("\n  Core Memory");
                for entry in &entries {
                    println!("\n  [{}] (~{} tokens)", entry.key, entry.token_count);
                    for line in entry.value.lines() {
                        println!("    {line}");
                    }
                }
                println!();
            }
        }
        Some(MemoryCommand::Search { query }) => {
            let (people, commitments, preferences, events) = tokio::join!(
                db.search_people(&query),
                db.search_commitments(&query),
                db.search_preferences(&query),
                db.search_events(&query),
            );

            let mut found = false;
            println!();

            if let Ok(ref items) = people {
                if !items.is_empty() {
                    found = true;
                    println!("  People:");
                    for p in items {
                        println!(
                            "    {} — {} {}",
                            p.canonical_name,
                            p.relationship.as_deref().unwrap_or(""),
                            p.notes.as_deref().unwrap_or("")
                        );
                    }
                    println!();
                }
            }

            if let Ok(ref items) = commitments {
                if !items.is_empty() {
                    found = true;
                    println!("  Commitments:");
                    for c in items {
                        println!(
                            "    #{} [{}] {} {}",
                            c.id,
                            c.status,
                            c.description,
                            c.due_date.as_deref().unwrap_or("")
                        );
                    }
                    println!();
                }
            }

            if let Ok(ref items) = preferences {
                if !items.is_empty() {
                    found = true;
                    println!("  Preferences:");
                    for p in items {
                        println!("    {}: {}", p.category, p.value);
                    }
                    println!();
                }
            }

            if let Ok(ref items) = events {
                if !items.is_empty() {
                    found = true;
                    println!("  Events:");
                    for e in items {
                        println!(
                            "    #{} {} {}",
                            e.id,
                            e.description,
                            e.event_date.as_deref().unwrap_or("")
                        );
                    }
                    println!();
                }
            }

            if !found {
                println!("  No results for \"{query}\"\n");
            }
        }
        Some(MemoryCommand::People) => {
            let items = db.list_people().await?;
            if items.is_empty() {
                println!("\n  No tracked people.\n");
            } else {
                println!("\n  People ({}):", items.len());
                for p in &items {
                    println!(
                        "    {} — {}",
                        p.canonical_name,
                        p.relationship.as_deref().unwrap_or("no relationship set")
                    );
                    if let Some(notes) = &p.notes {
                        println!("      {notes}");
                    }
                }
                println!();
            }
        }
        Some(MemoryCommand::Commitments { status }) => {
            let items = db.list_commitments(&status).await?;
            if items.is_empty() {
                println!("\n  No {status} commitments.\n");
            } else {
                println!("\n  Commitments — {status} ({}):", items.len());
                for c in &items {
                    let due = c
                        .due_date
                        .as_deref()
                        .map(|d| format!(" (due {d})"))
                        .unwrap_or_default();
                    println!("    #{}: {}{due}", c.id, c.description);
                }
                println!();
            }
        }
        Some(MemoryCommand::Preferences) => {
            let items = db.list_preferences().await?;
            if items.is_empty() {
                println!("\n  No stored preferences.\n");
            } else {
                println!("\n  Preferences ({}):", items.len());
                for p in &items {
                    println!("    {}: {}", p.category, p.value);
                }
                println!();
            }
        }
        Some(MemoryCommand::Events) => {
            let items = db.list_events().await?;
            if items.is_empty() {
                println!("\n  No stored events.\n");
            } else {
                println!("\n  Events ({}):", items.len());
                for e in &items {
                    let date = e
                        .event_date
                        .as_deref()
                        .map(|d| format!(" [{d}]"))
                        .unwrap_or_default();
                    println!("    #{}: {}{date}", e.id, e.description);
                }
                println!();
            }
        }
        Some(MemoryCommand::Reset { block }) => {
            let section = CORE_MEMORY_SECTIONS
                .iter()
                .find(|(k, _)| *k == block.as_str());

            if let Some(&(_, default_value)) = section {
                let reset_value = if block == "user_summary" {
                    // Re-seed from user.md if available
                    let user_md_path = ctx.home_dir.join("user.md");
                    let content = std::fs::read_to_string(&user_md_path).ok();
                    let from_file = content
                        .as_deref()
                        .filter(|s| !s.starts_with("# Tell Mika about yourself"));
                    from_file.unwrap_or(default_value).to_string()
                } else {
                    default_value.to_string()
                };

                db.set_core_memory(&block, &reset_value).await?;
                println!("\n  Reset [{block}] to default.\n");
            } else {
                let allowed = core_memory_section_names();
                println!(
                    "\n  Invalid block '{block}'. Allowed: {}\n",
                    allowed.join(", ")
                );
            }
        }
    }

    // Database shutdown happens automatically via Drop on ctx
    Ok(())
}
