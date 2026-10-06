use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::sync::OnceLock;
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Style as SyntectStyle, ThemeSet},
    parsing::SyntaxSet,
};

use crate::{filesystem::file_type::Language, preview::text};

pub fn highlight(content: &str, language: Option<Language>) -> Vec<Line<'static>> {
    let Some(language) = language else {
        return text::plain_lines(content);
    };
    let Some(syntax_name) = language.syntect_name() else {
        return text::plain_lines(content);
    };

    let resources = syntax_resources();
    let Some(theme) = resources
        .themes
        .themes
        .get("base16-ocean.dark")
        .or_else(|| resources.themes.themes.values().next())
    else {
        return text::plain_lines(content);
    };

    let syntax = resources
        .syntaxes
        .find_syntax_by_name(syntax_name)
        .or_else(|| {
            resources
                .syntaxes
                .find_syntax_by_extension(language.label())
        })
        .unwrap_or_else(|| resources.syntaxes.find_syntax_plain_text());

    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut lines = Vec::new();

    for line in content.split('\n') {
        let line = line.trim_end_matches('\r');
        let ranges = match highlighter.highlight_line(line, &resources.syntaxes) {
            Ok(ranges) => ranges,
            Err(_) => return text::plain_lines(content),
        };
        lines.push(Line::from(
            ranges
                .into_iter()
                .map(|(style, text)| Span::styled(text.to_string(), convert_style(style)))
                .collect::<Vec<_>>(),
        ));
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
}

struct SyntaxResources {
    syntaxes: SyntaxSet,
    themes: ThemeSet,
}

fn syntax_resources() -> &'static SyntaxResources {
    static RESOURCES: OnceLock<SyntaxResources> = OnceLock::new();
    RESOURCES.get_or_init(|| SyntaxResources {
        syntaxes: SyntaxSet::load_defaults_newlines(),
        themes: ThemeSet::load_defaults(),
    })
}

fn convert_style(style: SyntectStyle) -> Style {
    let mut ratatui_style = Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));

    if style.font_style.contains(FontStyle::BOLD) {
        ratatui_style = ratatui_style.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        ratatui_style = ratatui_style.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        ratatui_style = ratatui_style.add_modifier(Modifier::UNDERLINED);
    }

    ratatui_style
}
