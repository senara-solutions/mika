use anyhow::Result;

use crate::cli::{MemoryArgs, MemoryCommand};
use crate::init;

pub async fn run(args: MemoryArgs) -> Result<()> {
    let ctx = init::init_db_only()?;
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
    }

    ctx.async_db.shutdown();
    Ok(())
}
