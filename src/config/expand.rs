use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::model::{RawTerminalProfile, TerminalProfile};

pub fn expand_profile(raw: RawTerminalProfile, repo_root: &Path) -> Result<TerminalProfile> {
    let command = expand_string(&raw.command).with_context(|| {
        format!(
            "Invalid profile {:?}: command contains an invalid environment reference",
            raw.name
        )
    })?;
    let args = raw
        .args
        .iter()
        .map(|arg| expand_string(arg))
        .collect::<Result<Vec<_>>>()
        .with_context(|| format!("Invalid profile {:?}: argument expansion failed", raw.name))?;
    let cwd = raw
        .cwd
        .as_deref()
        .map(|cwd| expand_path(cwd, repo_root))
        .transpose()
        .with_context(|| format!("Invalid profile {:?}: cwd expansion failed", raw.name))?;
    let env = raw
        .env
        .iter()
        .map(|(key, value)| Ok((key.clone(), expand_string(value)?)))
        .collect::<Result<HashMap<_, _>>>()
        .with_context(|| {
            format!(
                "Invalid profile {:?}: environment value expansion failed",
                raw.name
            )
        })?;

    Ok(TerminalProfile {
        name: raw.name,
        command,
        args,
        cwd,
        env,
        auto_start: raw.auto_start,
        restart_on_exit: raw.restart_on_exit,
    })
}

pub fn expand_string(value: &str) -> Result<String> {
    let expanded = shellexpand::full(value)
        .map_err(|error| anyhow::anyhow!("unable to expand {value:?}: {error}"))?;
    Ok(expanded.into_owned())
}

pub fn expand_path(value: &str, repo_root: &Path) -> Result<PathBuf> {
    let expanded = expand_string(value)?;
    let path = PathBuf::from(expanded);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(repo_root.join(path))
    }
}
