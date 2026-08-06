use std::{collections::HashSet, path::Path};

use anyhow::{Context, Result};

use super::model::{ConfigFile, ResolvedConfig, TerminalProfile};

pub const SUPPORTED_CONFIG_VERSION: u32 = 1;
pub const RESERVED_FILES_TAB: &str = "Files";

pub fn validate_raw_config(config: &ConfigFile, source_name: &str) -> Result<()> {
    if config.version != SUPPORTED_CONFIG_VERSION {
        anyhow::bail!(
            "Invalid {source_name}: unsupported version {}, expected {}",
            config.version,
            SUPPORTED_CONFIG_VERSION
        );
    }

    if let Some(ignored_directories) = &config.workspace.ignored_directories {
        for directory in ignored_directories {
            if directory.trim().is_empty() {
                anyhow::bail!("Invalid {source_name}: ignored_directories entries cannot be empty");
            }
            if directory.contains('/') || directory.contains('\\') {
                anyhow::bail!(
                    "Invalid {source_name}: ignored_directories entries must be directory names, not paths: {:?}",
                    directory
                );
            }
        }
    }

    let mut names = HashSet::new();
    for profile in &config.tabs {
        let name = profile.name.trim();
        if name.is_empty() {
            anyhow::bail!("Invalid {source_name}: tab name cannot be empty");
        }
        if name == RESERVED_FILES_TAB {
            anyhow::bail!(
                "Invalid {source_name}: tab name {:?} is reserved",
                RESERVED_FILES_TAB
            );
        }
        if !names.insert(name.to_string()) {
            anyhow::bail!("Invalid {source_name}: duplicate tab name {:?}", name);
        }
        if profile.command.trim().is_empty() {
            anyhow::bail!(
                "Invalid {source_name}: profile {:?} command cannot be empty",
                profile.name
            );
        }
    }

    Ok(())
}

pub fn validate_resolved_config(config: &ResolvedConfig, repo_root: &Path) -> Result<()> {
    if !repo_root.is_dir() {
        anyhow::bail!("Missing repository root: {}", repo_root.display());
    }

    for profile in &config.tabs {
        validate_profile(profile)?;
    }

    if let Some(default_tab) = config.workspace.default_tab.as_deref() {
        let exists = default_tab == RESERVED_FILES_TAB
            || config
                .tabs
                .iter()
                .any(|profile| profile.name == default_tab);
        if !exists {
            anyhow::bail!(
                "Invalid configuration: default tab {:?} does not exist",
                default_tab
            );
        }
    }

    Ok(())
}

fn validate_profile(profile: &TerminalProfile) -> Result<()> {
    if profile.name.trim().is_empty() {
        anyhow::bail!("Invalid profile: tab name cannot be empty");
    }
    if profile.name == RESERVED_FILES_TAB {
        anyhow::bail!(
            "Invalid profile {:?}: {:?} is a reserved tab name",
            profile.name,
            RESERVED_FILES_TAB
        );
    }
    if profile.command.trim().is_empty() {
        anyhow::bail!(
            "Invalid profile {:?}: command cannot be empty",
            profile.name
        );
    }
    if let Some(cwd) = &profile.cwd {
        if !cwd.is_dir() {
            anyhow::bail!(
                "Invalid profile {:?}: working directory does not exist: {}",
                profile.name,
                cwd.display()
            );
        }
    }
    Ok(())
}

pub fn parse_config_toml(contents: &str, source_name: &str) -> Result<ConfigFile> {
    let config = toml::from_str::<ConfigFile>(contents)
        .with_context(|| format!("Invalid {source_name}: malformed TOML"))?;
    validate_raw_config(&config, source_name)?;
    Ok(config)
}
