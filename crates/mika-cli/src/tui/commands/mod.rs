pub mod autocomplete;
pub mod completers;
pub mod handlers;

use autocomplete::{CompletionContext, CompletionItem};

/// Signature for argument completer functions.
pub type CompleterFn = fn(&str, usize, &CompletionContext) -> (Vec<CompletionItem>, &'static str);

/// Definition of a slash command.
pub struct SlashCommand {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub args_hint: Option<&'static str>,
    /// Returns completion candidates for the given argument prefix and position.
    pub completer: Option<CompleterFn>,
}

/// All available slash commands. Adding a new command = append here + add match arm in handlers::dispatch.
pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "help",
        aliases: &["h", "?"],
        description: "List available commands",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "clear",
        aliases: &[],
        description: "Clear chat and start a new session",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "exit",
        aliases: &["quit", "q"],
        description: "Quit mika",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "compact",
        aliases: &[],
        description: "Compact conversation history",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "memory",
        aliases: &["mem"],
        description: "Show core memory blocks",
        args_hint: Some("[search <query>]"),
        completer: Some(completers::complete_memory),
    },
    SlashCommand {
        name: "reminders",
        aliases: &["remind"],
        description: "List active reminders",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "tasks",
        aliases: &[],
        description: "List or cancel pending tasks",
        args_hint: Some("[cancel <id>]"),
        completer: None,
    },
    SlashCommand {
        name: "status",
        aliases: &["stat"],
        description: "Show system health info",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "soul",
        aliases: &[],
        description: "Display current soul.md",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "config",
        aliases: &["cfg"],
        description: "Show or set config",
        args_hint: Some("[set <key> <value>]"),
        completer: Some(completers::complete_config),
    },
    SlashCommand {
        name: "model",
        aliases: &[],
        description: "Show or switch model",
        args_hint: Some("[<name|alias>]"),
        completer: Some(completers::complete_model),
    },
    SlashCommand {
        name: "provider",
        aliases: &[],
        description: "Show or switch LLM provider",
        args_hint: Some("[anthropic|openai|groq|ollama|...]"),
        completer: Some(completers::complete_provider),
    },
    SlashCommand {
        name: "export",
        aliases: &[],
        description: "Export conversation to markdown",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "skills",
        aliases: &[],
        description: "List loaded skills",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "skill",
        aliases: &[],
        description: "Show skill details",
        args_hint: Some("<name>"),
        completer: Some(completers::complete_skill),
    },
    SlashCommand {
        name: "switch",
        aliases: &["agent"],
        description: "Switch to a different agent",
        args_hint: Some("<name>"),
        completer: Some(completers::complete_switch),
    },
    SlashCommand {
        name: "agents",
        aliases: &[],
        description: "List all agents",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "teams",
        aliases: &[],
        description: "List all teams",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "team",
        aliases: &[],
        description: "Run a team workflow",
        args_hint: Some("<name> \"<goal>\""),
        completer: Some(completers::complete_team),
    },
    SlashCommand {
        name: "think",
        aliases: &["t"],
        description: "Set thinking level or think once",
        args_hint: Some("[low|medium|high|off] [prompt]"),
        completer: Some(completers::complete_think),
    },
    SlashCommand {
        name: "attach",
        aliases: &["img"],
        description: "Attach an image file",
        args_hint: Some("<path>"),
        completer: Some(completers::complete_attach),
    },
    SlashCommand {
        name: "verbose",
        aliases: &["v"],
        description: "Toggle verbose mode (team mode)",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "inbox",
        aliases: &[],
        description: "Toggle inbox/audit mode (hide/show internal messages)",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "undo",
        aliases: &[],
        description: "Undo last exchange and reverse memory changes",
        args_hint: None,
        completer: None,
    },
    SlashCommand {
        name: "rewind",
        aliases: &[],
        description: "Rewind N exchanges or to a message ID",
        args_hint: Some("[<count> | to <message_id>]"),
        completer: None,
    },
    SlashCommand {
        name: "restart",
        aliases: &[],
        description: "Restart the agent worker after a crash (mika#1149)",
        args_hint: None,
        completer: None,
    },
];

/// Resolve a thinking level keyword to (budget_tokens, level_name).
pub fn resolve_thinking_level(word: &str) -> Option<(u32, &'static str)> {
    match word.to_lowercase().as_str() {
        "low" => Some((5_000, "low")),
        "medium" | "med" => Some((10_000, "medium")),
        "high" => Some((50_000, "high")),
        _ => None,
    }
}

/// Filter commands by prefix (case-insensitive). Matches command names and aliases.
pub fn filter_commands(prefix: &str) -> Vec<&'static SlashCommand> {
    let prefix_lower = prefix.to_lowercase();
    COMMANDS
        .iter()
        .filter(|cmd| {
            cmd.name.starts_with(&prefix_lower)
                || cmd.aliases.iter().any(|a| a.starts_with(&prefix_lower))
        })
        .collect()
}

/// Parse a slash command string into (command_name, args).
pub fn parse_command(input: &str) -> (&str, &str) {
    let trimmed = input.trim_start_matches('/').trim();
    match trimmed.split_once(char::is_whitespace) {
        Some((cmd, args)) => (cmd, args.trim()),
        None => (trimmed, ""),
    }
}

/// Find a command by name (exact match on name or alias).
pub fn find_command(name: &str) -> Option<&'static SlashCommand> {
    let lower = name.to_lowercase();
    COMMANDS
        .iter()
        .find(|cmd| cmd.name == lower || cmd.aliases.iter().any(|a| *a == lower))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_exact_match() {
        let results = filter_commands("help");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "help");
    }

    #[test]
    fn test_filter_prefix() {
        let results = filter_commands("me");
        let names: Vec<_> = results.iter().map(|c| c.name).collect();
        assert!(names.contains(&"memory"));
        // "me" does not match "model" (starts with "mo")
        assert!(!names.contains(&"model"));

        // "m" matches both memory and model
        let results = filter_commands("m");
        let names: Vec<_> = results.iter().map(|c| c.name).collect();
        assert!(names.contains(&"memory"));
        assert!(names.contains(&"model"));
    }

    #[test]
    fn test_filter_alias() {
        let results = filter_commands("q");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "exit");
    }

    #[test]
    fn test_filter_no_match() {
        let results = filter_commands("zzz");
        assert!(results.is_empty());
    }

    #[test]
    fn test_filter_empty_returns_all() {
        let results = filter_commands("");
        assert_eq!(results.len(), COMMANDS.len());
    }

    #[test]
    fn test_parse_command_no_args() {
        let (cmd, args) = parse_command("/status");
        assert_eq!(cmd, "status");
        assert_eq!(args, "");
    }

    #[test]
    fn test_parse_command_with_args() {
        let (cmd, args) = parse_command("/memory search hello world");
        assert_eq!(cmd, "memory");
        assert_eq!(args, "search hello world");
    }

    #[test]
    fn test_parse_command_extra_whitespace() {
        let (cmd, args) = parse_command("/  config   set  key  value ");
        assert_eq!(cmd, "config");
        assert_eq!(args, "set  key  value");
    }

    #[test]
    fn test_find_command_by_name() {
        let cmd = find_command("model").unwrap();
        assert_eq!(cmd.name, "model");
    }

    #[test]
    fn test_find_command_by_alias() {
        let cmd = find_command("q").unwrap();
        assert_eq!(cmd.name, "exit");
    }

    #[test]
    fn test_find_command_not_found() {
        assert!(find_command("nonexistent").is_none());
    }
}
