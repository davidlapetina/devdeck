use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::{
    merge::merge_configs,
    model::{ConfigFile, ResolvedConfig},
    validate::parse_config_toml,
};

pub fn load_config(repo_root: &Path) -> Result<ResolvedConfig> {
    let global_path = global_config_path();
    let project_path = repo_root.join(".devdeck.toml");
    load_from_paths(global_path.as_deref(), Some(&project_path), repo_root)
}

pub fn load_from_paths(
    global_path: Option<&Path>,
    project_path: Option<&Path>,
    repo_root: &Path,
) -> Result<ResolvedConfig> {
    let global = global_path
        .filter(|path| path.exists())
        .map(|path| read_config(path, "global config"))
        .transpose()?;
    let project = project_path
        .filter(|path| path.exists())
        .map(|path| read_config(path, ".devdeck.toml"))
        .transpose()?;

    merge_configs(global, project, repo_root)
}

fn read_config(path: &Path, source_name: &str) -> Result<ConfigFile> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Unable to read {source_name}: {}", path.display()))?;
    parse_config_toml(&contents, source_name)
}

fn global_config_path() -> Option<PathBuf> {
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(config_home).join("devdeck/config.toml"));
    }

    home_dir().map(|home| home.join(".config/devdeck/config.toml"))
}

fn home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        env::var_os("USERPROFILE").map(PathBuf::from)
    } else {
        env::var_os("HOME").map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn loads_project_config() {
        let temp = TempDir::new().unwrap();
        let config = temp.path().join(".devdeck.toml");
        fs::write(
            &config,
            r#"
version = 1
[[tabs]]
name = "Shell"
command = "sh"
"#,
        )
        .unwrap();

        let resolved = load_from_paths(None, Some(&config), temp.path()).unwrap();

        assert_eq!(resolved.tabs.len(), 1);
        assert_eq!(resolved.tabs[0].name, "Shell");
    }

    #[test]
    fn reports_duplicate_names_with_source_context() {
        let temp = TempDir::new().unwrap();
        let config = temp.path().join(".devdeck.toml");
        fs::write(
            &config,
            r#"
version = 1
[[tabs]]
name = "Tests"
command = "sh"
[[tabs]]
name = "Tests"
command = "bash"
"#,
        )
        .unwrap();

        let error = load_from_paths(None, Some(&config), temp.path())
            .unwrap_err()
            .to_string();

        assert!(error.contains("duplicate tab name \"Tests\""));
    }

    #[test]
    fn rejects_reserved_files_name() {
        let temp = TempDir::new().unwrap();
        let config = temp.path().join(".devdeck.toml");
        fs::write(
            &config,
            r#"
version = 1
[[tabs]]
name = "Files"
command = "sh"
"#,
        )
        .unwrap();

        let error = load_from_paths(None, Some(&config), temp.path())
            .unwrap_err()
            .to_string();

        assert!(error.contains("reserved"));
    }

    #[test]
    fn rejects_missing_cwd() {
        let temp = TempDir::new().unwrap();
        let config = temp.path().join(".devdeck.toml");
        fs::write(
            &config,
            r#"
version = 1
[[tabs]]
name = "Backend"
command = "sh"
cwd = "missing"
"#,
        )
        .unwrap();

        let error = load_from_paths(None, Some(&config), temp.path())
            .unwrap_err()
            .to_string();

        assert!(error.contains("working directory does not exist"));
    }
}
