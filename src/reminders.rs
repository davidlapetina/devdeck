use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

const REMINDER_DIR: &str = ".devdeck";
const REMINDER_FILE: &str = "reminders.toml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reminder {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    pub due: String,
    pub done: bool,
    pub created_at: String,
}

impl Reminder {
    pub fn due_date(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(&self.due, "%Y-%m-%d").ok()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct ReminderFile {
    #[serde(default)]
    reminders: Vec<Reminder>,
}

#[derive(Debug, Clone)]
pub struct ReminderStore {
    pub root: PathBuf,
    pub path: PathBuf,
    pub reminders: Vec<Reminder>,
}

impl ReminderStore {
    pub fn empty(workspace_root: &Path) -> Self {
        let root = reminder_root(workspace_root);
        Self {
            path: root.join(REMINDER_DIR).join(REMINDER_FILE),
            root,
            reminders: Vec::new(),
        }
    }

    pub fn load(workspace_root: &Path) -> Result<Self> {
        let root = reminder_root(workspace_root);
        let path = root.join(REMINDER_DIR).join(REMINDER_FILE);
        if !path.exists() {
            return Ok(Self {
                root,
                path,
                reminders: Vec::new(),
            });
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Unable to read reminders: {}", path.display()))?;
        let file = toml::from_str::<ReminderFile>(&contents)
            .with_context(|| format!("Unable to parse reminders: {}", path.display()))?;
        let mut store = Self {
            root,
            path,
            reminders: file.reminders,
        };
        store.sort();
        Ok(store)
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Unable to create reminder directory: {}", parent.display())
            })?;
        }
        let file = ReminderFile {
            reminders: self.reminders.clone(),
        };
        let contents = toml::to_string_pretty(&file).context("Unable to serialize reminders")?;
        fs::write(&self.path, contents)
            .with_context(|| format!("Unable to write reminders: {}", self.path.display()))
    }

    pub fn relative_path(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf())
    }

    pub fn absolute_path(&self, reminder: &Reminder) -> PathBuf {
        if reminder.path.is_absolute() {
            reminder.path.clone()
        } else {
            self.root.join(&reminder.path)
        }
    }

    pub fn add(&mut self, title: String, path: &Path, due: String) -> Result<()> {
        let reminder = Reminder {
            id: format!("r{}", Local::now().timestamp_micros()),
            title,
            path: self.relative_path(path),
            due,
            done: false,
            created_at: Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        };
        self.reminders.push(reminder);
        self.sort();
        self.save()
    }

    pub fn toggle_done(&mut self, index: usize) -> Result<()> {
        if let Some(reminder) = self.reminders.get_mut(index) {
            reminder.done = !reminder.done;
            self.sort();
            self.save()?;
        }
        Ok(())
    }

    pub fn remove(&mut self, index: usize) -> Result<()> {
        if index < self.reminders.len() {
            self.reminders.remove(index);
            self.save()?;
        }
        Ok(())
    }

    pub fn active_for_path(&self, path: &Path) -> Vec<&Reminder> {
        let relative = self.relative_path(path);
        self.reminders
            .iter()
            .filter(|reminder| !reminder.done && reminder.path == relative)
            .collect()
    }

    pub fn has_active_under_path(&self, path: &Path) -> bool {
        let relative = self.relative_path(path);
        self.reminders
            .iter()
            .filter(|reminder| !reminder.done)
            .any(|reminder| reminder.path == relative || reminder.path.starts_with(&relative))
    }

    fn sort(&mut self) {
        self.reminders.sort_by(|left, right| {
            left.done
                .cmp(&right.done)
                .then_with(|| left.due.cmp(&right.due))
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| left.title.cmp(&right.title))
        });
    }
}

pub fn parse_due_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok()
}

fn reminder_root(workspace_root: &Path) -> PathBuf {
    if let Some(existing) = find_upward(workspace_root, |path| {
        path.join(REMINDER_DIR).join(REMINDER_FILE).exists()
    }) {
        return existing;
    }
    if let Some(project_root) = find_upward(workspace_root, |path| {
        path.join(".devdeck.toml").exists() || path.join(".git").exists()
    }) {
        return project_root;
    }
    workspace_root.to_path_buf()
}

fn find_upward(root: &Path, predicate: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut current = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    loop {
        if predicate(&current) {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn discovers_parent_reminder_file_from_subfolder() {
        let temp = TempDir::new().unwrap();
        let sub = temp.path().join("src");
        fs::create_dir_all(temp.path().join(REMINDER_DIR)).unwrap();
        fs::create_dir_all(&sub).unwrap();
        fs::write(
            temp.path().join(REMINDER_DIR).join(REMINDER_FILE),
            "[[reminders]]\nid = \"r1\"\ntitle = \"Check\"\npath = \"src/lib.rs\"\ndue = \"2026-08-20\"\ndone = false\ncreated_at = \"2026-08-12T00:00:00+00:00\"\n",
        )
        .unwrap();

        let store = ReminderStore::load(&sub).unwrap();

        assert_eq!(store.root, temp.path().canonicalize().unwrap());
        assert_eq!(store.reminders.len(), 1);
    }

    #[test]
    fn validates_due_date_format() {
        assert!(parse_due_date("2026-08-20").is_some());
        assert!(parse_due_date("08/20/2026").is_none());
    }
}
