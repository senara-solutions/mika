use super::{SlashCommand, filter_commands};

/// Tracks autocomplete popup state for slash commands.
pub struct AutocompleteState {
    pub visible: bool,
    pub items: Vec<&'static SlashCommand>,
    pub selected: usize,
}

impl AutocompleteState {
    pub fn new() -> Self {
        Self {
            visible: false,
            items: Vec::new(),
            selected: 0,
        }
    }

    /// Update suggestions based on current input text.
    /// Shows popup when input starts with "/" and has no space yet (command-level completion).
    pub fn update(&mut self, input: &str) {
        if input.starts_with('/') && !input[1..].contains(' ') {
            let prefix = &input[1..];
            self.items = filter_commands(prefix);
            self.visible = !self.items.is_empty();
            // Clamp selected index
            if self.selected >= self.items.len() {
                self.selected = 0;
            }
        } else {
            self.dismiss();
        }
    }

    /// Move selection to the next item (wraps around).
    pub fn next(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1) % self.items.len();
        }
    }

    /// Move selection to the previous item (wraps around).
    pub fn previous(&mut self) {
        if !self.items.is_empty() {
            self.selected = self.selected.checked_sub(1).unwrap_or(self.items.len() - 1);
        }
    }

    /// Get the name of the currently selected command, if any.
    pub fn selected_name(&self) -> Option<&'static str> {
        if self.visible {
            self.items.get(self.selected).map(|cmd| cmd.name)
        } else {
            None
        }
    }

    /// Dismiss the popup without clearing input.
    pub fn dismiss(&mut self) {
        self.visible = false;
        self.items.clear();
        self.selected = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_shows_all_on_slash() {
        let mut state = AutocompleteState::new();
        state.update("/");
        assert!(state.visible);
        assert!(!state.items.is_empty());
    }

    #[test]
    fn test_update_filters_on_prefix() {
        let mut state = AutocompleteState::new();
        state.update("/he");
        assert!(state.visible);
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].name, "help");
    }

    #[test]
    fn test_update_hides_on_no_match() {
        let mut state = AutocompleteState::new();
        state.update("/zzz");
        assert!(!state.visible);
    }

    #[test]
    fn test_update_hides_after_space() {
        let mut state = AutocompleteState::new();
        state.update("/memory search");
        assert!(!state.visible);
    }

    #[test]
    fn test_update_hides_on_no_slash() {
        let mut state = AutocompleteState::new();
        state.update("hello");
        assert!(!state.visible);
    }

    #[test]
    fn test_next_wraps_around() {
        let mut state = AutocompleteState::new();
        state.update("/he");
        assert_eq!(state.selected, 0);
        state.next();
        // Only 1 item, wraps back to 0
        assert_eq!(state.selected, 0);

        // Multiple items
        state.update("/");
        let count = state.items.len();
        for _ in 0..count {
            state.next();
        }
        assert_eq!(state.selected, 0); // wrapped back
    }

    #[test]
    fn test_previous_wraps_around() {
        let mut state = AutocompleteState::new();
        state.update("/");
        let count = state.items.len();
        state.previous(); // 0 -> last
        assert_eq!(state.selected, count - 1);
    }

    #[test]
    fn test_selected_name() {
        let mut state = AutocompleteState::new();
        state.update("/");
        assert_eq!(state.selected_name(), Some("help")); // first command
    }

    #[test]
    fn test_dismiss_resets_state() {
        let mut state = AutocompleteState::new();
        state.update("/");
        assert!(state.visible);
        state.dismiss();
        assert!(!state.visible);
        assert!(state.items.is_empty());
        assert_eq!(state.selected, 0);
    }
}
