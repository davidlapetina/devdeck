use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result};

pub const DEFAULT_IGNORED_DIRECTORIES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".dart_tool",
    "dist",
    "coverage",
    ".idea",
];

pub fn default_ignored_directories() -> Vec<String> {
    DEFAULT_IGNORED_DIRECTORIES
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub children: Vec<TreeNode>,
}

#[derive(Debug, Clone)]
pub struct VisibleEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub struct FlatEntry {
    pub path: PathBuf,
    pub display_path: String,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone)]
pub struct FileTree {
    pub root: PathBuf,
    pub root_node: TreeNode,
}

impl FileTree {
    pub fn scan(
        root: impl AsRef<Path>,
        show_hidden: bool,
        ignored_directories: &[String],
    ) -> Result<Self> {
        let root = root
            .as_ref()
            .canonicalize()
            .with_context(|| format!("Unable to resolve path: {}", root.as_ref().display()))?;

        if !root.is_dir() {
            anyhow::bail!("Path is not a directory: {}", root.display());
        }

        let root_node = scan_node(&root, &root, show_hidden, ignored_directories)?;
        Ok(Self { root, root_node })
    }

    pub fn visible_entries(&self, expanded: &HashSet<PathBuf>) -> Vec<VisibleEntry> {
        let mut entries = Vec::new();
        collect_visible(&self.root_node, expanded, 0, &mut entries);
        entries
    }

    pub fn all_entries(&self) -> Vec<FlatEntry> {
        let mut entries = Vec::new();
        collect_flat(&self.root_node, &self.root, &mut entries);
        entries
    }

    pub fn contains_path(&self, path: &Path) -> bool {
        find_node(&self.root_node, path).is_some()
    }

    pub fn node(&self, path: &Path) -> Option<&TreeNode> {
        find_node(&self.root_node, path)
    }

    pub fn nearest_existing_ancestor(&self, path: &Path) -> PathBuf {
        let mut candidate = path.to_path_buf();
        loop {
            if self.contains_path(&candidate) {
                return candidate;
            }
            if !candidate.pop() {
                return self.root.clone();
            }
        }
    }
}

pub fn is_ignored_directory(name: &str, ignored_directories: &[String]) -> bool {
    ignored_directories
        .iter()
        .any(|ignored| ignored.eq_ignore_ascii_case(name))
}

pub fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.') && name != "." && name != ".."
}

fn scan_node(
    path: &Path,
    root: &Path,
    show_hidden: bool,
    ignored_directories: &[String],
) -> Result<TreeNode> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Unable to read metadata: {}", path.display()))?;
    let is_dir = metadata.is_dir();
    let name = if path == root {
        ".".to_string()
    } else {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    };

    let mut children = Vec::new();
    if is_dir {
        let mut entries = Vec::new();
        for entry in fs::read_dir(path)
            .with_context(|| format!("Unable to read directory: {}", path.display()))?
        {
            let entry = entry
                .with_context(|| format!("Unable to read directory entry in {}", path.display()))?;
            let child_path = entry.path();
            let child_name = entry.file_name().to_string_lossy().to_string();
            let metadata = match fs::symlink_metadata(&child_path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            let child_is_dir = metadata.is_dir();

            if !show_hidden && is_hidden_name(&child_name) {
                continue;
            }
            if child_is_dir && is_ignored_directory(&child_name, ignored_directories) {
                continue;
            }

            entries.push((child_path, child_name, child_is_dir));
        }

        entries.sort_by(|left, right| {
            right
                .2
                .cmp(&left.2)
                .then_with(|| {
                    left.1
                        .to_ascii_lowercase()
                        .cmp(&right.1.to_ascii_lowercase())
                })
                .then_with(|| left.1.cmp(&right.1))
        });

        for (child_path, _, _) in entries {
            if let Ok(child) = scan_node(&child_path, root, show_hidden, ignored_directories) {
                children.push(child);
            }
        }
    }

    Ok(TreeNode {
        path: path.to_path_buf(),
        name,
        is_dir,
        size: metadata.len(),
        modified: metadata.modified().ok(),
        children,
    })
}

fn collect_visible(
    node: &TreeNode,
    expanded: &HashSet<PathBuf>,
    depth: usize,
    entries: &mut Vec<VisibleEntry>,
) {
    entries.push(VisibleEntry {
        path: node.path.clone(),
        name: node.name.clone(),
        is_dir: node.is_dir,
        depth,
        size: node.size,
        modified: node.modified,
    });

    if node.is_dir && expanded.contains(&node.path) {
        for child in &node.children {
            collect_visible(child, expanded, depth + 1, entries);
        }
    }
}

fn collect_flat(node: &TreeNode, root: &Path, entries: &mut Vec<FlatEntry>) {
    let display_path = if node.path == root {
        ".".to_string()
    } else {
        node.path
            .strip_prefix(root)
            .unwrap_or(&node.path)
            .to_string_lossy()
            .to_string()
    };

    entries.push(FlatEntry {
        path: node.path.clone(),
        display_path,
        name: node.name.clone(),
        is_dir: node.is_dir,
    });

    for child in &node.children {
        collect_flat(child, root, entries);
    }
}

fn find_node<'a>(node: &'a TreeNode, path: &Path) -> Option<&'a TreeNode> {
    if node.path == path {
        return Some(node);
    }

    node.children
        .iter()
        .find_map(|child| find_node(child, path))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::TempDir;

    use super::*;

    fn touch(path: &Path) {
        fs::write(path, "").unwrap();
    }

    fn scan_default(root: &Path, show_hidden: bool) -> FileTree {
        FileTree::scan(root, show_hidden, &default_ignored_directories()).unwrap()
    }

    #[test]
    fn sorts_directories_before_files_alphabetically() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("zeta")).unwrap();
        fs::create_dir(temp.path().join("alpha")).unwrap();
        touch(&temp.path().join("beta.txt"));
        touch(&temp.path().join("aardvark.txt"));

        let tree = scan_default(temp.path(), false);
        let names: Vec<_> = tree
            .root_node
            .children
            .iter()
            .map(|node| node.name.as_str())
            .collect();

        assert_eq!(names, ["alpha", "zeta", "aardvark.txt", "beta.txt"]);
    }

    #[test]
    fn filters_hidden_names_when_disabled() {
        let temp = TempDir::new().unwrap();
        touch(&temp.path().join(".hidden"));
        touch(&temp.path().join("visible"));

        let tree = scan_default(temp.path(), false);
        let names: Vec<_> = tree
            .root_node
            .children
            .iter()
            .map(|node| node.name.as_str())
            .collect();

        assert_eq!(names, ["visible"]);
    }

    #[test]
    fn includes_hidden_names_when_enabled() {
        let temp = TempDir::new().unwrap();
        touch(&temp.path().join(".hidden"));
        touch(&temp.path().join("visible"));

        let tree = scan_default(temp.path(), true);
        let names: Vec<_> = tree
            .root_node
            .children
            .iter()
            .map(|node| node.name.as_str())
            .collect();

        assert_eq!(names, [".hidden", "visible"]);
    }

    #[test]
    fn filters_hardcoded_ignored_directories() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::create_dir(temp.path().join("node_modules")).unwrap();
        fs::create_dir(temp.path().join("build")).unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();

        let tree = scan_default(temp.path(), true);
        let names: Vec<_> = tree
            .root_node
            .children
            .iter()
            .map(|node| node.name.as_str())
            .collect();

        assert_eq!(names, ["build", "src"]);
    }

    #[test]
    fn uses_configured_ignored_directories() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("cache")).unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();

        let tree = FileTree::scan(temp.path(), true, &["cache".to_string()]).unwrap();
        let names: Vec<_> = tree
            .root_node
            .children
            .iter()
            .map(|node| node.name.as_str())
            .collect();

        assert_eq!(names, ["src"]);
    }

    #[test]
    fn finds_nearest_existing_ancestor() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("docs")).unwrap();
        touch(&temp.path().join("docs/readme.md"));

        let tree = scan_default(temp.path(), false);
        let root = temp.path().canonicalize().unwrap();
        let ancestor = tree.nearest_existing_ancestor(&root.join("docs/missing.md"));

        assert_eq!(ancestor, root.join("docs"));
    }
}
