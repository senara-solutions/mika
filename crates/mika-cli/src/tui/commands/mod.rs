pub mod autocomplete;
pub mod handlers;

/// Definition of a slash command.
pub struct SlashCommand {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub args_hint: Option<&'static str>,
}

/// All available slash commands. Adding a new command = append here + add match arm in handlers::dispatch.
pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "help",
        aliases: &["h", "?"],
        description: "List available commands",
        args_hint: None,
    },
    SlashCommand {
        name: "clear",
        aliases: &[],
        description: "Clear chat display (--all for DB)",
        args_hint: Some("[--all]"),
    },
    SlashCommand {
        name: "exit",
        aliases: &["quit", "q"],
        description: "Quit mika",
        args_hint: None,
    },
    SlashCommand {
        name: "compact",
        aliases: &[],
        description: "Compact conversation history",
        args_hint: None,
    },
    SlashCommand {
        name: "memory",
        aliases: &["mem"],
        description: "Show core memory blocks",
        args_hint: Some("[search <query>]"),
    },
    SlashCommand {
        name: "reminders",
        aliases: &["remind"],
        description: "List active reminders",
        args_hint: None,
    },
    SlashCommand {
        name: "status",
        aliases: &["stat"],
        description: "Show system health info",
        args_hint: None,
    },
    SlashCommand {
        name: "soul",
        aliases: &[],
        description: "Display current soul.md",
        args_hint: None,
    },
    SlashCommand {
        name: "config",
        aliases: &["cfg"],
        description: "Show current config",
        args_hint: None,
    },
    SlashCommand {
        name: "model",
        aliases: &[],
        description: "Show active model",
        args_hint: None,
    },
    SlashCommand {
        name: "export",
        aliases: &[],
        description: "Export conversation to markdown",
        args_hint: None,
    },
    SlashCommand {
        name: "skills",
        aliases: &[],
        description: "List loaded skills",
        args_hint: None,
    },
    SlashCommand {
        name: "skill",
        aliases: &[],
        description: "Show skill details",
        args_hint: Some("<name>"),
    },
    SlashCommand {
        name: "switch",
        aliases: &["agent"],
        description: "Switch to a different agent",
        args_hint: Some("<name>"),
    },
    SlashCommand {
        name: "agents",
        aliases: &[],
        description: "List all agents",
        args_hint: None,
    },
];

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
}
