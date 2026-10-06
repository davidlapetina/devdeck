use std::{collections::HashMap, path::PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MousePolicy {
    #[default]
    Auto,
    Selection,
    Navigation,
}

impl MousePolicy {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Selection => "selection",
            Self::Navigation => "navigation",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Auto => Self::Selection,
            Self::Selection => Self::Navigation,
            Self::Navigation => Self::Auto,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfigFile {
    pub version: u32,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub tabs: Vec<RawTerminalProfile>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct WorkspaceConfig {
    pub default_tab: Option<String>,
    pub ignored_directories: Option<Vec<String>>,
    /// Key chord used to select the previous tab (for example `Alt-Left`).
    pub previous_tab_key: Option<String>,
    /// Key chord used to select the next tab (for example `Alt-Right`).
    pub next_tab_key: Option<String>,
    /// Mouse behavior: auto, selection, or navigation.
    pub mouse_policy: Option<MousePolicy>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawTerminalProfile {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default)]
    pub restart_on_exit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalProfile {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub auto_start: bool,
    pub restart_on_exit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub workspace: WorkspaceConfig,
    pub tabs: Vec<TerminalProfile>,
}
