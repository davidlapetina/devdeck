use std::path::{Path, PathBuf};

use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};

use crate::filesystem::tree::FlatEntry;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub path: PathBuf,
    pub display_path: String,
    pub score: i64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub active: bool,
    pub query: String,
    pub selected: usize,
    pub results: Vec<SearchResult>,
}

impl SearchState {
    pub fn open(&mut self, entries: &[FlatEntry]) {
        self.active = true;
        self.query.clear();
        self.selected = 0;
        self.update(entries);
    }

    pub fn cancel(&mut self) {
        self.active = false;
        self.query.clear();
        self.selected = 0;
        self.results.clear();
    }

    pub fn push(&mut self, ch: char, entries: &[FlatEntry]) {
        self.query.push(ch);
        self.update(entries);
    }

    pub fn backspace(&mut self, entries: &[FlatEntry]) {
        self.query.pop();
        self.update(entries);
    }

    pub fn move_down(&mut self) {
        if !self.results.is_empty() {
            self.selected = (self.selected + 1).min(self.results.len() - 1);
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.results
            .get(self.selected)
            .map(|result| result.path.as_path())
    }

    pub fn update(&mut self, entries: &[FlatEntry]) {
        self.results = search_entries(entries, &self.query);
        if self.selected >= self.results.len() {
            self.selected = self.results.len().saturating_sub(1);
        }
    }
}

pub fn search_entries(entries: &[FlatEntry], query: &str) -> Vec<SearchResult> {
    let matcher = SkimMatcherV2::default();
    let mut results = entries
        .iter()
        .filter(|entry| entry.display_path != ".")
        .filter_map(|entry| {
            let score = if query.trim().is_empty() {
                Some(0)
            } else {
                matcher
                    .fuzzy_match(&entry.display_path, query)
                    .or_else(|| matcher.fuzzy_match(&entry.name, query))
            }?;

            Some(SearchResult {
                path: entry.path.clone(),
                display_path: entry.display_path.clone(),
                score,
                is_dir: entry.is_dir,
            })
        })
        .collect::<Vec<_>>();

    results.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.display_path.len().cmp(&right.display_path.len()))
            .then_with(|| left.display_path.cmp(&right.display_path))
    });
    results.truncate(100);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_search_prefers_better_filename_matches() {
        let entries = vec![
            FlatEntry {
                path: PathBuf::from("/repo/docs/architecture.md"),
                display_path: "docs/architecture.md".to_string(),
                name: "architecture.md".to_string(),
                is_dir: false,
            },
            FlatEntry {
                path: PathBuf::from("/repo/archive/archived-report.md"),
                display_path: "archive/archived-report.md".to_string(),
                name: "archived-report.md".to_string(),
                is_dir: false,
            },
            FlatEntry {
                path: PathBuf::from("/repo/docs/system-architecture.md"),
                display_path: "docs/system-architecture.md".to_string(),
                name: "system-architecture.md".to_string(),
                is_dir: false,
            },
        ];

        let results = search_entries(&entries, "architecture");

        assert_eq!(results[0].display_path, "docs/architecture.md");
        assert!(results
            .iter()
            .any(|result| result.display_path == "docs/system-architecture.md"));
    }
}
