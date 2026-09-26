use std::path::Path;

use mika_agent::skills::SkillRegistry;

use super::{SlashCommand, filter_commands};

/// A single completion candidate.
#[derive(Clone, Debug)]
pub struct CompletionItem {
    /// The value to insert (e.g., "sonnet", "mika", "~/Documents").
    pub value: String,
    /// Optional description shown alongside (e.g., "Claude Sonnet 4.6").
    pub description: Option<String>,
}

/// Context passed to argument completer functions.
pub struct CompletionContext<'a> {
    pub home_dir: &'a Path,
    pub global_home: &'a Path,
    pub skills: &'a SkillRegistry,
    pub current_agent: &'a str,
    pub cwd: &'a Path,
    /// Full args string (everything after the command name), for multi-arg completers.
    pub args_str: &'a str,
    /// Current active provider in the TUI (may differ from config on disk).
    pub provider: mika_common::llm::ProviderKind,
}

/// What kind of completion is active.
pub enum CompletionMode {
    /// No popup visible.
    Hidden,
    /// Completing a command name after "/".
    Command {
        items: Vec<&'static SlashCommand>,
        selected: usize,
    },
    /// Completing an argument for a known command.
    Argument {
        items: Vec<CompletionItem>,
        selected: usize,
        /// Popup title for this argument context (e.g., " Models ", " Agents ").
        title: &'static str,
        /// Whether the popup should use a wider layout (e.g., for file paths).
        wide: bool,
    },
}

/// Tracks autocomplete popup state for slash commands and arguments.
pub struct AutocompleteState {
    pub mode: CompletionMode,
}

impl AutocompleteState {
    pub fn new() -> Self {
        Self {
            mode: CompletionMode::Hidden,
        }
    }

    /// Whether the popup is visible.
    pub fn visible(&self) -> bool {
        !matches!(self.mode, CompletionMode::Hidden)
    }

    /// Update suggestions based on current input text (command-level only).
    /// Shows popup when input starts with "/" and has no space yet.
    pub fn update_command(&mut self, input: &str) {
        if input.starts_with('/') && !input[1..].contains(' ') {
            let prefix = &input[1..];
            let items = filter_commands(prefix);
            if items.is_empty() {
                self.dismiss();
            } else {
                let selected = match &self.mode {
                    CompletionMode::Command { selected: prev, .. } => {
                        (*prev).min(items.len().saturating_sub(1))
                    }
                    _ => 0,
                };
                self.mode = CompletionMode::Command { items, selected };
            }
        } else {
            self.dismiss();
        }
    }

    /// Show argument completions for a command.
    pub fn show_arguments(&mut self, items: Vec<CompletionItem>, title: &'static str, wide: bool) {
        if items.is_empty() {
            self.dismiss();
        } else {
            self.mode = CompletionMode::Argument {
                items,
                selected: 0,
                title,
                wide,
            };
        }
    }

    /// Move selection to the next item (wraps around).
    pub fn next(&mut self) {
        match &mut self.mode {
            CompletionMode::Command { items, selected } => {
                if !items.is_empty() {
                    *selected = (*selected + 1) % items.len();
                }
            }
            CompletionMode::Argument {
                items, selected, ..
            } => {
                if !items.is_empty() {
                    *selected = (*selected + 1) % items.len();
                }
            }
            CompletionMode::Hidden => {}
        }
    }

    /// Move selection to the previous item (wraps around).
    pub fn previous(&mut self) {
        match &mut self.mode {
            CompletionMode::Command { items, selected } => {
                if !items.is_empty() {
                    *selected = selected.checked_sub(1).unwrap_or(items.len() - 1);
                }
            }
            CompletionMode::Argument {
                items, selected, ..
            } => {
                if !items.is_empty() {
                    *selected = selected.checked_sub(1).unwrap_or(items.len() - 1);
                }
            }
            CompletionMode::Hidden => {}
        }
    }

    /// Get the name of the currently selected command (command mode only).
    #[allow(dead_code)] // used in tests
    pub fn selected_command(&self) -> Option<&'static SlashCommand> {
        match &self.mode {
            CompletionMode::Command { items, selected } => items.get(*selected).copied(),
            _ => None,
        }
    }

    /// Get the value of the currently selected item.
    #[allow(dead_code)] // used in tests
    pub fn selected_value(&self) -> Option<&str> {
        match &self.mode {
            CompletionMode::Command { items, selected } => items.get(*selected).map(|cmd| cmd.name),
            CompletionMode::Argument {
                items, selected, ..
            } => items.get(*selected).map(|item| item.value.as_str()),
            CompletionMode::Hidden => None,
        }
    }

    /// Get all current item values (for computing common prefix).
    pub fn item_values(&self) -> Vec<&str> {
        match &self.mode {
            CompletionMode::Command { items, .. } => items.iter().map(|cmd| cmd.name).collect(),
            CompletionMode::Argument { items, .. } => {
                items.iter().map(|item| item.value.as_str()).collect()
            }
            CompletionMode::Hidden => Vec::new(),
        }
    }

    /// Get the selected index.
    pub fn selected_index(&self) -> usize {
        match &self.mode {
            CompletionMode::Command { selected, .. } => *selected,
            CompletionMode::Argument { selected, .. } => *selected,
            CompletionMode::Hidden => 0,
        }
    }

    /// Get the number of items.
    pub fn item_count(&self) -> usize {
        match &self.mode {
            CompletionMode::Command { items, .. } => items.len(),
            CompletionMode::Argument { items, .. } => items.len(),
            CompletionMode::Hidden => 0,
        }
    }

    /// Get the popup title.
    pub fn title(&self) -> &str {
        match &self.mode {
            CompletionMode::Command { .. } => " Commands ",
            CompletionMode::Argument { title, .. } => title,
            CompletionMode::Hidden => "",
        }
    }

    /// Dismiss the popup without clearing input.
    pub fn dismiss(&mut self) {
        self.mode = CompletionMode::Hidden;
    }
}

/// Compute the longest common prefix of a set of strings.
pub fn longest_common_prefix(strings: &[&str]) -> String {
    if strings.is_empty() {
        return String::new();
    }
    if strings.len() == 1 {
        return strings[0].to_string();
    }

    let first = strings[0];
    let mut len = first.len();
    for s in &strings[1..] {
        len = len.min(s.len());
        for (i, (a, b)) in first.bytes().zip(s.bytes()).enumerate() {
            if !a.eq_ignore_ascii_case(&b) {
                len = len.min(i);
                break;
            }
        }
    }
    first[..len].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_shows_all_on_slash() {
        let mut state = AutocompleteState::new();
        state.update_command("/");
        assert!(state.visible());
        assert!(state.item_count() > 0);
    }

    #[test]
    fn test_update_filters_on_prefix() {
        let mut state = AutocompleteState::new();
        state.update_command("/he");
        assert!(state.visible());
        assert_eq!(state.item_count(), 1);
        assert_eq!(state.selected_value(), Some("help"));
    }

    #[test]
    fn test_update_hides_on_no_match() {
        let mut state = AutocompleteState::new();
        state.update_command("/zzz");
        assert!(!state.visible());
    }

    #[test]
    fn test_update_hides_after_space() {
        let mut state = AutocompleteState::new();
        state.update_command("/memory search");
        assert!(!state.visible());
    }

    #[test]
    fn test_update_hides_on_no_slash() {
        let mut state = AutocompleteState::new();
        state.update_command("hello");
        assert!(!state.visible());
    }

    #[test]
    fn test_next_wraps_around() {
        let mut state = AutocompleteState::new();
        state.update_command("/he");
        assert_eq!(state.selected_index(), 0);
        state.next();
        // Only 1 item, wraps back to 0
        assert_eq!(state.selected_index(), 0);

        // Multiple items
        state.update_command("/");
        let count = state.item_count();
        for _ in 0..count {
            state.next();
        }
        assert_eq!(state.selected_index(), 0); // wrapped back
    }

    #[test]
    fn test_previous_wraps_around() {
        let mut state = AutocompleteState::new();
        state.update_command("/");
        let count = state.item_count();
        state.previous(); // 0 -> last
        assert_eq!(state.selected_index(), count - 1);
    }

    #[test]
    fn test_selected_command() {
        let mut state = AutocompleteState::new();
        state.update_command("/");
        let cmd = state.selected_command().unwrap();
        assert_eq!(cmd.name, "help"); // first command
    }

    #[test]
    fn test_dismiss_resets_state() {
        let mut state = AutocompleteState::new();
        state.update_command("/");
        assert!(state.visible());
        state.dismiss();
        assert!(!state.visible());
        assert_eq!(state.item_count(), 0);
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn test_longest_common_prefix_single() {
        assert_eq!(longest_common_prefix(&["hello"]), "hello");
    }

    #[test]
    fn test_longest_common_prefix_multiple() {
        assert_eq!(longest_common_prefix(&["compact", "config"]), "co");
    }

    #[test]
    fn test_longest_common_prefix_exact() {
        assert_eq!(longest_common_prefix(&["model"]), "model");
    }

    #[test]
    fn test_longest_common_prefix_empty() {
        assert_eq!(longest_common_prefix(&[]), "");
    }

    #[test]
    fn test_longest_common_prefix_no_common() {
        assert_eq!(longest_common_prefix(&["abc", "xyz"]), "");
    }

    #[test]
    fn test_argument_mode() {
        let mut state = AutocompleteState::new();
        state.show_arguments(
            vec![
                CompletionItem {
                    value: "sonnet".to_string(),
                    description: Some("Claude Sonnet 4.6".to_string()),
                },
                CompletionItem {
                    value: "opus".to_string(),
                    description: Some("Claude Opus 4.6".to_string()),
                },
            ],
            " Models ",
            false,
        );
        assert!(state.visible());
        assert_eq!(state.item_count(), 2);
        assert_eq!(state.selected_value(), Some("sonnet"));
        assert_eq!(state.title(), " Models ");

        state.next();
        assert_eq!(state.selected_value(), Some("opus"));
    }

    #[test]
    fn test_argument_mode_empty_dismisses() {
        let mut state = AutocompleteState::new();
        state.show_arguments(vec![], " Models ", false);
        assert!(!state.visible());
    }

    #[test]
    fn test_selected_preserves_across_filter() {
        let mut state = AutocompleteState::new();
        state.update_command("/");
        // Move to item 2
        state.next();
        state.next();
        let prev_selected = state.selected_index();

        // Re-filter with same prefix — should clamp
        state.update_command("/");
        assert_eq!(state.selected_index(), prev_selected);
    }
}
