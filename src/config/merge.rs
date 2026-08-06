use std::path::Path;

use anyhow::Result;

use super::{
    expand::expand_profile,
    model::{ConfigFile, ResolvedConfig, TerminalProfile, WorkspaceConfig},
    validate::validate_resolved_config,
};

pub fn merge_configs(
    global: Option<ConfigFile>,
    project: Option<ConfigFile>,
    repo_root: &Path,
) -> Result<ResolvedConfig> {
    let mut workspace = WorkspaceConfig::default();
    let mut tabs = Vec::<TerminalProfile>::new();

    if let Some(global) = global {
        workspace = global.workspace;
        for profile in global.tabs {
            tabs.push(expand_profile(profile, repo_root)?);
        }
    }

    if let Some(project) = project {
        if project.workspace.default_tab.is_some() {
            workspace.default_tab = project.workspace.default_tab;
        }
        if project.workspace.ignored_directories.is_some() {
            workspace.ignored_directories = project.workspace.ignored_directories;
        }

        for raw_profile in project.tabs {
            let profile = expand_profile(raw_profile, repo_root)?;
            if let Some(existing) = tabs
                .iter_mut()
                .find(|existing| existing.name == profile.name)
            {
                *existing = profile;
            } else {
                tabs.push(profile);
            }
        }
    }

    let resolved = ResolvedConfig { workspace, tabs };
    validate_resolved_config(&resolved, repo_root)?;
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use tempfile::TempDir;

    use super::*;
    use crate::config::validate::parse_config_toml;

    #[test]
    fn project_profile_replaces_global_profile_by_name() {
        let temp = TempDir::new().unwrap();
        let global = parse_config_toml(
            r#"
version = 1
[[tabs]]
name = "Shell"
command = "sh"
args = ["-l"]
"#,
            "global config",
        )
        .unwrap();
        let project = parse_config_toml(
            r#"
version = 1
[[tabs]]
name = "Shell"
command = "bash"
"#,
            ".devdeck.toml",
        )
        .unwrap();

        let resolved = merge_configs(Some(global), Some(project), temp.path()).unwrap();

        assert_eq!(resolved.tabs.len(), 1);
        assert_eq!(resolved.tabs[0].command, "bash");
        assert!(resolved.tabs[0].args.is_empty());
    }

    #[test]
    fn relative_cwd_resolves_against_repository_root() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("tools")).unwrap();
        let project = parse_config_toml(
            r#"
version = 1
[[tabs]]
name = "Tools"
command = "sh"
cwd = "tools"
"#,
            ".devdeck.toml",
        )
        .unwrap();

        let resolved = merge_configs(None, Some(project), temp.path()).unwrap();

        assert_eq!(resolved.tabs[0].cwd, Some(temp.path().join("tools")));
    }

    #[test]
    fn expands_environment_variables_in_commands_and_args() {
        let temp = TempDir::new().unwrap();
        env::set_var("DEVDECK_TEST_COMMAND", "sh");
        env::set_var("DEVDECK_TEST_ARG", "-l");
        let project = parse_config_toml(
            r#"
version = 1
[[tabs]]
name = "Env"
command = "${DEVDECK_TEST_COMMAND}"
args = ["$DEVDECK_TEST_ARG"]
"#,
            ".devdeck.toml",
        )
        .unwrap();

        let resolved = merge_configs(None, Some(project), temp.path()).unwrap();

        assert_eq!(resolved.tabs[0].command, "sh");
        assert_eq!(resolved.tabs[0].args, ["-l"]);
    }

    #[test]
    fn project_ignored_directories_replace_global_value() {
        let temp = TempDir::new().unwrap();
        let global = parse_config_toml(
            r#"
version = 1
[workspace]
ignored_directories = ["target", "node_modules"]
"#,
            "global config",
        )
        .unwrap();
        let project = parse_config_toml(
            r#"
version = 1
[workspace]
ignored_directories = ["cache"]
"#,
            ".devdeck.toml",
        )
        .unwrap();

        let resolved = merge_configs(Some(global), Some(project), temp.path()).unwrap();

        assert_eq!(
            resolved.workspace.ignored_directories,
            Some(vec!["cache".to_string()])
        );
    }

    #[test]
    fn project_can_disable_ignored_directories() {
        let temp = TempDir::new().unwrap();
        let global = parse_config_toml(
            r#"
version = 1
[workspace]
ignored_directories = ["target"]
"#,
            "global config",
        )
        .unwrap();
        let project = parse_config_toml(
            r#"
version = 1
[workspace]
ignored_directories = []
"#,
            ".devdeck.toml",
        )
        .unwrap();

        let resolved = merge_configs(Some(global), Some(project), temp.path()).unwrap();

        assert_eq!(resolved.workspace.ignored_directories, Some(Vec::new()));
    }
}
