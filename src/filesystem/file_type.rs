use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Markdown,
    Json,
    Yaml,
    Toml,
    Shell,
    Dart,
    JavaScript,
    TypeScript,
    Python,
    Xml,
    Env,
    Text,
}

impl Language {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Markdown => "markdown",
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
            Self::Shell => "shell",
            Self::Dart => "dart",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Python => "python",
            Self::Xml => "xml",
            Self::Env => "env",
            Self::Text => "text",
        }
    }

    pub fn syntect_name(self) -> Option<&'static str> {
        match self {
            Self::Rust => Some("Rust"),
            Self::Markdown => Some("Markdown"),
            Self::Json => Some("JSON"),
            Self::Yaml => Some("YAML"),
            Self::Toml => Some("TOML"),
            Self::Shell => Some("Bourne Again Shell (bash)"),
            Self::Dart => Some("Dart"),
            Self::JavaScript => Some("JavaScript"),
            Self::TypeScript => Some("TypeScript"),
            Self::Python => Some("Python"),
            Self::Xml => Some("XML"),
            Self::Env | Self::Text => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedFileType {
    Directory,
    Markdown,
    Source(Language),
    Text(Language),
    Binary,
    TooLarge,
}

impl DetectedFileType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::Markdown => "markdown",
            Self::Source(language) | Self::Text(language) => language.label(),
            Self::Binary => "binary",
            Self::TooLarge => "too large",
        }
    }
}

pub fn detect_path(path: &Path, is_dir: bool) -> DetectedFileType {
    if is_dir {
        return DetectedFileType::Directory;
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "md" | "markdown" | "mdown" => DetectedFileType::Markdown,
        "rs" => DetectedFileType::Source(Language::Rust),
        "json" => DetectedFileType::Source(Language::Json),
        "yaml" | "yml" => DetectedFileType::Source(Language::Yaml),
        "toml" => DetectedFileType::Source(Language::Toml),
        "sh" | "bash" | "zsh" | "fish" => DetectedFileType::Source(Language::Shell),
        "dart" => DetectedFileType::Source(Language::Dart),
        "js" | "mjs" | "cjs" | "jsx" => DetectedFileType::Source(Language::JavaScript),
        "ts" | "tsx" => DetectedFileType::Source(Language::TypeScript),
        "py" | "pyw" => DetectedFileType::Source(Language::Python),
        "xml" => DetectedFileType::Source(Language::Xml),
        "txt" => DetectedFileType::Text(Language::Text),
        "env" => DetectedFileType::Text(Language::Env),
        _ => match file_name.as_str() {
            ".env" | ".env.local" | ".envrc" => DetectedFileType::Text(Language::Env),
            "makefile" | "dockerfile" | "justfile" | "rakefile" | "gemfile" => {
                DetectedFileType::Source(Language::Shell)
            }
            "license" | "readme" | "authors" | "contributors" => {
                DetectedFileType::Text(Language::Text)
            }
            _ => DetectedFileType::Text(Language::Text),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_requested_source_types() {
        assert_eq!(
            detect_path(Path::new("src/main.rs"), false),
            DetectedFileType::Source(Language::Rust)
        );
        assert_eq!(
            detect_path(Path::new("pubspec.yaml"), false),
            DetectedFileType::Source(Language::Yaml)
        );
        assert_eq!(
            detect_path(Path::new("lib/main.dart"), false),
            DetectedFileType::Source(Language::Dart)
        );
        assert_eq!(
            detect_path(Path::new("README.md"), false),
            DetectedFileType::Markdown
        );
    }

    #[test]
    fn detects_extensionless_configuration_files() {
        assert_eq!(
            detect_path(Path::new(".env"), false),
            DetectedFileType::Text(Language::Env)
        );
        assert_eq!(
            detect_path(Path::new("Dockerfile"), false),
            DetectedFileType::Source(Language::Shell)
        );
    }
}
