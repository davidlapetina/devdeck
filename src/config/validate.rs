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

    for (name, binding) in [
        (
            "previous_tab_key",
            config.workspace.previous_tab_key.as_deref(),
        ),
        ("next_tab_key", config.workspace.next_tab_key.as_deref()),
    ] {
        if let Some(binding) = binding {
            validate_tab_key_binding(binding)
                .with_context(|| format!("Invalid {source_name}: invalid workspace.{name}"))?;
        }
    }
    if let (Some(previous), Some(next)) = (
        config.workspace.previous_tab_key.as_deref(),
        config.workspace.next_tab_key.as_deref(),
    ) {
        validate_distinct_tab_key_bindings(previous, next)
            .with_context(|| format!("Invalid {source_name}: invalid workspace tab bindings"))?;
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

fn validate_tab_key_binding(binding: &str) -> Result<()> {
    canonical_tab_key_binding(binding).map(|_| ())
}

fn canonical_tab_key_binding(binding: &str) -> Result<&'static str> {
    match binding.trim().to_ascii_lowercase().as_str() {
        "alt-left" => Ok("alt-left"),
        "alt-right" => Ok("alt-right"),
        "ctrl-left" | "control-left" => Ok("ctrl-left"),
        "ctrl-right" | "control-right" => Ok("ctrl-right"),
        _ => anyhow::bail!("expected Alt-Left, Alt-Right, Ctrl-Left, or Ctrl-Right"),
    }
}

fn validate_distinct_tab_key_bindings(previous: &str, next: &str) -> Result<()> {
    if canonical_tab_key_binding(previous)? == canonical_tab_key_binding(next)? {
        anyhow::bail!("workspace.previous_tab_key and workspace.next_tab_key must be different");
    }
    Ok(())
}

pub fn validate_resolved_config(config: &ResolvedConfig, repo_root: &Path) -> Result<()> {
    if !repo_root.is_dir() {
        anyhow::bail!("Missing repository root: {}", repo_root.display());
    }

    let previous = config
        .workspace
        .previous_tab_key
        .as_deref()
        .unwrap_or("Alt-Left");
    let next = config
        .workspace
        .next_tab_key
        .as_deref()
        .unwrap_or("Alt-Right");
    validate_distinct_tab_key_bindings(previous, next)
        .context("Invalid configuration: tab navigation bindings")?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_tab_navigation_binding() {
        let error = parse_config_toml(
            r#"
version = 1
[workspace]
next_tab_key = "Alt-N"
"#,
            ".devdeck.toml",
        )
        .unwrap_err();
        let error = format!("{error:#}");

        assert!(error.contains("workspace.next_tab_key"));
    }

    #[test]
    fn rejects_unknown_mouse_policy() {
        let error = parse_config_toml(
            r#"
version = 1
[workspace]
mouse_policy = "sometimes"
"#,
            ".devdeck.toml",
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("mouse_policy"));
    }

    #[test]
    fn rejects_bare_arrow_tab_navigation_binding() {
        let error = parse_config_toml(
            r#"
version = 1
[workspace]
previous_tab_key = "Left"
"#,
            ".devdeck.toml",
        )
        .unwrap_err();
        let error = format!("{error:#}");

        assert!(error.contains("workspace.previous_tab_key"));
        assert!(error.contains("Alt-Left"));
    }

    #[test]
    fn rejects_duplicate_tab_navigation_bindings_in_one_file() {
        let error = parse_config_toml(
            r#"
version = 1
[workspace]
previous_tab_key = "Ctrl-Left"
next_tab_key = "Control-Left"
"#,
            ".devdeck.toml",
        )
        .unwrap_err();
        let error = format!("{error:#}");

        assert!(error.contains("must be different"));
    }
}
