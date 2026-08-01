use std::{env, path::Path};

use crate::{config::TerminalProfile, session::CommandSpec};

pub fn command_spec_from_profile(profile: &TerminalProfile, repo_root: &Path) -> CommandSpec {
    let cwd = profile
        .cwd
        .clone()
        .unwrap_or_else(|| repo_root.to_path_buf());
    let mut env_values = profile.env.clone();
    env_values.insert("TERM".to_string(), "xterm-256color".to_string());
    env_values.insert("COLORTERM".to_string(), "truecolor".to_string());
    env_values.insert("DEVDECK".to_string(), "1".to_string());

    CommandSpec {
        executable: profile.command.clone(),
        args: profile.args.clone(),
        cwd,
        env: env_values,
    }
}

pub fn parent_shell_profile(repo_root: &Path) -> TerminalProfile {
    let command = env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(windows) {
            "cmd".to_string()
        } else {
            "sh".to_string()
        }
    });

    TerminalProfile {
        name: "Shell".to_string(),
        command,
        args: Vec::new(),
        cwd: Some(repo_root.to_path_buf()),
        env: Default::default(),
        auto_start: true,
        restart_on_exit: false,
    }
}
