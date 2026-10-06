use ratatui::{prelude::*, widgets::*};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::App,
    session::TerminalPromptState,
    tabs::{ActivityState, TabContent, TerminalTabState},
};

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let project = app
        .root_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("devdeck");
    let block = Block::default()
        .title(format!(" devdeck - {project} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let layout = strip_layout(app, inner.width as usize);
    let mut spans = Vec::new();
    if layout.leading_hidden {
        spans.push(Span::styled(
            LEADING_OVERFLOW,
            Style::default().fg(Color::DarkGray),
        ));
    }
    for (index, label) in layout.indices.iter().copied().zip(layout.labels.iter()) {
        let style = if index == app.active_tab {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(label.clone(), style));
    }
    if layout.trailing_hidden {
        spans.push(Span::styled(
            TRAILING_OVERFLOW,
            Style::default().fg(Color::DarkGray),
        ));
    } else if layout.show_cta {
        spans.push(Span::styled(
            NEW_TAB_CTA,
            Style::default().fg(Color::DarkGray),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

pub fn hit_test(area: Rect, app: &App, column: u16, row: u16) -> Option<usize> {
    let inner = Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if row < area.y
        || row >= area.y.saturating_add(area.height)
        || column < inner.x
        || column >= inner.x.saturating_add(inner.width)
    {
        return None;
    }

    let layout = strip_layout(app, inner.width as usize);
    let mut x = inner.x;
    if layout.leading_hidden {
        x = x.saturating_add(2);
    }
    for (index, label) in layout.indices.into_iter().zip(layout.labels) {
        let width = UnicodeWidthStr::width(label.as_str()).min(u16::MAX as usize) as u16;
        let end = x.saturating_add(width);
        if column >= x && column < end {
            return Some(index);
        }
        x = end;
    }
    None
}

#[derive(Debug, PartialEq, Eq)]
struct TabStripLayout {
    indices: Vec<usize>,
    labels: Vec<String>,
    leading_hidden: bool,
    trailing_hidden: bool,
    show_cta: bool,
}

const LEADING_OVERFLOW: &str = "‹ ";
const TRAILING_OVERFLOW: &str = "›";
const NEW_TAB_CTA: &str = "[+ c/Ctrl-b c]";

fn strip_layout(app: &App, width: usize) -> TabStripLayout {
    if app.tabs.is_empty() || width == 0 {
        return TabStripLayout {
            indices: Vec::new(),
            labels: Vec::new(),
            leading_hidden: false,
            trailing_hidden: false,
            show_cta: false,
        };
    }

    let active = app.active_tab.min(app.tabs.len() - 1);
    let labels = (0..app.tabs.len())
        .map(|index| tab_label(app, index))
        .collect::<Vec<_>>();
    let mut best = None;
    for start in 0..=active {
        for end in (active + 1)..=app.tabs.len() {
            let label_width = labels[start..end]
                .iter()
                .map(|label| UnicodeWidthStr::width(label.as_str()))
                .sum::<usize>();
            let decorations = usize::from(start > 0) * UnicodeWidthStr::width(LEADING_OVERFLOW)
                + usize::from(end < app.tabs.len()) * UnicodeWidthStr::width(TRAILING_OVERFLOW);
            let used = label_width + decorations;
            if used > width {
                continue;
            }
            let count = end - start;
            if best.is_none_or(|(best_start, best_end, best_used)| {
                count > best_end - best_start
                    || (count == best_end - best_start && used > best_used)
            }) {
                best = Some((start, end, used));
            }
        }
    }

    if let Some((start, end, used)) = best {
        let show_cta = end == app.tabs.len()
            && used.saturating_add(UnicodeWidthStr::width(NEW_TAB_CTA)) <= width;
        return TabStripLayout {
            indices: (start..end).collect(),
            labels: labels[start..end].to_vec(),
            leading_hidden: start > 0,
            trailing_hidden: end < app.tabs.len(),
            show_cta,
        };
    }

    // Even when the terminal is narrower than the active label, preserve a
    // visible (possibly truncated) representation of that tab. Overflow
    // markers are secondary and are only included when one display cell still
    // remains for the active label.
    let mut remaining = width;
    let leading_width = UnicodeWidthStr::width(LEADING_OVERFLOW);
    let trailing_width = UnicodeWidthStr::width(TRAILING_OVERFLOW);
    let leading_hidden = active > 0 && remaining > leading_width;
    if leading_hidden {
        remaining -= leading_width;
    }
    let trailing_hidden = active + 1 < app.tabs.len() && remaining > trailing_width;
    if trailing_hidden {
        remaining -= trailing_width;
    }

    TabStripLayout {
        indices: vec![active],
        labels: vec![truncate_display_width(&labels[active], remaining)],
        leading_hidden,
        trailing_hidden,
        show_cta: false,
    }
}

fn truncate_display_width(value: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let content_width = max_width - UnicodeWidthChar::width('…').unwrap_or(1);
    let mut result = String::new();
    let mut used = 0;
    for ch in value.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > content_width {
            break;
        }
        result.push(ch);
        used += ch_width;
    }
    result.push('…');
    result
}

fn tab_label(app: &App, index: usize) -> String {
    let tab = &app.tabs[index];
    let state = match &tab.content {
        TabContent::Repository => "",
        TabContent::Terminal(terminal)
            if matches!(
                terminal.state,
                TerminalTabState::Exited { .. } | TerminalTabState::Failed { .. }
            ) =>
        {
            "!"
        }
        TabContent::Terminal(terminal)
            if terminal
                .session_id
                .and_then(|session_id| app.sessions.session(session_id))
                .is_some_and(|session| session.prompt_state == TerminalPromptState::AtPrompt) =>
        {
            ">"
        }
        TabContent::Terminal(_) => "",
    };
    let activity = if state.is_empty() {
        match tab.activity {
            ActivityState::None => "",
            ActivityState::OutputActive { .. } => "*",
            ActivityState::OutputQuiet => ".",
        }
    } else {
        ""
    };
    format!("[{} {}{}{}] ", index + 1, tab.title, activity, state)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::TempDir;

    use super::*;
    use crate::{
        config::{ResolvedConfig, TerminalProfile, WorkspaceConfig},
        tabs::{Tab, TabId},
    };

    fn app(temp: &TempDir) -> App {
        App::new(
            temp.path().to_path_buf(),
            false,
            false,
            ResolvedConfig {
                workspace: WorkspaceConfig::default(),
                tabs: Vec::new(),
            },
        )
        .unwrap()
    }

    fn terminal_profile(name: &str) -> TerminalProfile {
        TerminalProfile {
            name: name.to_string(),
            command: "sh".to_string(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            auto_start: false,
            restart_on_exit: false,
        }
    }

    #[test]
    fn hit_test_returns_tab_under_rendered_label() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        app.tabs.push(Tab::terminal_tab(
            TabId(2),
            terminal_profile("Shell"),
            false,
        ));
        let area = Rect::new(0, 0, 80, 3);
        let first_tab_width = UnicodeWidthStr::width("[1 Files] ") as u16;

        assert_eq!(hit_test(area, &app, 1, 1), Some(0));
        assert_eq!(hit_test(area, &app, 1, 0), Some(0));
        assert_eq!(hit_test(area, &app, 1 + first_tab_width, 1), Some(1));
        assert_eq!(hit_test(area, &app, 1 + first_tab_width, 2), Some(1));
        assert_eq!(hit_test(area, &app, 0, 1), None);
    }

    #[test]
    fn narrow_tab_strip_keeps_the_active_tab_visible() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        for (id, name) in [(2, "One"), (3, "Two"), (4, "Three"), (5, "Active")] {
            app.tabs
                .push(Tab::terminal_tab(TabId(id), terminal_profile(name), false));
        }
        app.active_tab = 4;

        let layout = strip_layout(&app, 18);

        assert!(layout.leading_hidden);
        assert!(!layout.trailing_hidden);
        assert!(layout.indices.contains(&app.active_tab));
    }

    #[test]
    fn overflow_hit_test_uses_the_same_visible_window_as_rendering() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        for (id, name) in [(2, "One"), (3, "Two"), (4, "Three"), (5, "Active")] {
            app.tabs
                .push(Tab::terminal_tab(TabId(id), terminal_profile(name), false));
        }
        app.active_tab = 4;
        let area = Rect::new(0, 0, 20, 3);

        assert_eq!(hit_test(area, &app, 3, 1), Some(4));
    }

    #[test]
    fn every_rendered_segment_fits_the_requested_width() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        for (id, name) in [
            (2, "日本語"),
            (3, "Very long terminal title"),
            (4, "Active 🚀 tab"),
        ] {
            app.tabs
                .push(Tab::terminal_tab(TabId(id), terminal_profile(name), false));
        }
        app.active_tab = 3;

        for width in 1..=80 {
            let layout = strip_layout(&app, width);
            let rendered_width = usize::from(layout.leading_hidden)
                * UnicodeWidthStr::width(LEADING_OVERFLOW)
                + layout
                    .labels
                    .iter()
                    .map(|label| UnicodeWidthStr::width(label.as_str()))
                    .sum::<usize>()
                + usize::from(layout.trailing_hidden) * UnicodeWidthStr::width(TRAILING_OVERFLOW)
                + usize::from(layout.show_cta) * UnicodeWidthStr::width(NEW_TAB_CTA);

            assert!(rendered_width <= width, "{rendered_width} > {width}");
            assert!(layout.indices.contains(&app.active_tab));
            assert_eq!(layout.indices.len(), layout.labels.len());
            assert!(layout.labels.iter().any(|label| !label.is_empty()));
        }
    }

    #[test]
    fn wide_strip_budgets_the_new_tab_cta() {
        let temp = TempDir::new().unwrap();
        let app = app(&temp);
        let label_width = UnicodeWidthStr::width(tab_label(&app, 0).as_str());
        let cta_width = UnicodeWidthStr::width(NEW_TAB_CTA);

        let without_room = strip_layout(&app, label_width + cta_width - 1);
        assert!(!without_room.show_cta);

        let with_room = strip_layout(&app, label_width + cta_width);
        assert!(with_room.show_cta);
    }
}
