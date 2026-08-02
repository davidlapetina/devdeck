use std::time::Instant;

use crate::{config::TerminalProfile, session::SessionId};

pub const RESERVED_FILES_TITLE: &str = "Files";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

#[derive(Debug, Clone)]
pub struct Tab {
    pub id: TabId,
    pub title: String,
    pub content: TabContent,
    pub activity: ActivityState,
    pub temporary: bool,
    pub return_to_files_on_exit: bool,
}

impl Tab {
    pub fn repository(id: TabId) -> Self {
        Self {
            id,
            title: RESERVED_FILES_TITLE.to_string(),
            content: TabContent::Repository,
            activity: ActivityState::None,
            temporary: false,
            return_to_files_on_exit: false,
        }
    }

    pub fn terminal_tab(id: TabId, profile: TerminalProfile, temporary: bool) -> Self {
        let title = profile.name.clone();
        Self {
            id,
            title,
            content: TabContent::Terminal(TerminalTab {
                profile,
                session_id: None,
                state: TerminalTabState::NotStarted,
                requires_restart: false,
                removed_from_config: false,
                restart_attempts: 0,
                last_started_at: None,
                pending_restart_at: None,
            }),
            activity: ActivityState::None,
            temporary,
            return_to_files_on_exit: false,
        }
    }

    pub fn terminal_mut(&mut self) -> Option<&mut TerminalTab> {
        match &mut self.content {
            TabContent::Terminal(tab) => Some(tab),
            TabContent::Repository => None,
        }
    }

    pub fn as_terminal(&self) -> Option<&TerminalTab> {
        match &self.content {
            TabContent::Terminal(tab) => Some(tab),
            TabContent::Repository => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TabContent {
    Repository,
    Terminal(TerminalTab),
}

#[derive(Debug, Clone)]
pub struct TerminalTab {
    pub profile: TerminalProfile,
    pub session_id: Option<SessionId>,
    pub state: TerminalTabState,
    pub requires_restart: bool,
    pub removed_from_config: bool,
    pub restart_attempts: u8,
    pub last_started_at: Option<Instant>,
    pub pending_restart_at: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalTabState {
    NotStarted,
    Starting,
    Running,
    Exited { exit_code: Option<i32> },
    Failed { message: String },
}

impl TerminalTabState {
    pub fn label(&self) -> String {
        match self {
            Self::NotStarted => "not started".to_string(),
            Self::Starting => "starting".to_string(),
            Self::Running => "running".to_string(),
            Self::Exited { exit_code } => match exit_code {
                Some(code) => format!("exited {code}"),
                None => "exited".to_string(),
            },
            Self::Failed { message } => format!("failed: {message}"),
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running | Self::Starting)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityState {
    None,
    OutputActive { last_output_at: Instant },
    OutputQuiet,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn profile(name: &str) -> TerminalProfile {
        TerminalProfile {
            name: name.to_string(),
            command: "sh".to_string(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            auto_start: false,
            restart_on_exit: false,
        }
    }

    #[test]
    fn repository_tab_is_files_and_not_temporary() {
        let tab = Tab::repository(TabId(1));

        assert_eq!(tab.title, RESERVED_FILES_TITLE);
        assert!(!tab.temporary);
        assert!(matches!(tab.content, TabContent::Repository));
    }

    #[test]
    fn terminal_tab_starts_not_started() {
        let tab = Tab::terminal_tab(TabId(2), profile("Shell"), false);

        assert_eq!(tab.title, "Shell");
        assert_eq!(
            tab.as_terminal().unwrap().state,
            TerminalTabState::NotStarted
        );
    }
}
