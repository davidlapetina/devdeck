use std::{collections::HashMap, path::PathBuf};

use serde::Deserialize;

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
