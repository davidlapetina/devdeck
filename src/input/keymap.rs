use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    MoveDown,
    MoveUp,
    ExpandOrEnter,
    CollapseOrParent,
    Activate,
    First,
    Last,
    Root,
    ToggleHidden,
    Quit,
    ToggleMarkdown,
    PreviewPageDown,
    PreviewPageUp,
    PreviewLineDown,
    PreviewLineUp,
    PreviewTop,
    PreviewBottom,
    PreviewLinkNext,
    PreviewLinkPrevious,
    Search,
    RefreshFile,
    RefreshTree,
    OpenEditor,
    OpenInDevdeckEditor,
    OpenOs,
    CopyRelative,
    CopyAbsolute,
    CtrlC,
}

pub fn map_key(event: KeyEvent) -> Option<KeyAction> {
    match event {
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => Some(KeyAction::CtrlC),
        KeyEvent {
            code: KeyCode::Char('d'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
        | KeyEvent {
            code: KeyCode::PageDown,
            ..
        } => Some(KeyAction::PreviewPageDown),
        KeyEvent {
            code: KeyCode::Char('u'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
        | KeyEvent {
            code: KeyCode::PageUp,
            ..
        } => Some(KeyAction::PreviewPageUp),
        KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Down,
            ..
        } => Some(KeyAction::MoveDown),
        KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Up, ..
        } => Some(KeyAction::MoveUp),
        KeyEvent {
            code: KeyCode::Char('l'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Right,
            ..
        } => Some(KeyAction::ExpandOrEnter),
        KeyEvent {
            code: KeyCode::Char('h'),
            modifiers: KeyModifiers::NONE,
            ..
        }
        | KeyEvent {
            code: KeyCode::Left,
            ..
        } => Some(KeyAction::CollapseOrParent),
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } => Some(KeyAction::Activate),
        KeyEvent {
            code: KeyCode::Char('g'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::First),
        KeyEvent {
            code: KeyCode::Char('G'),
            modifiers: KeyModifiers::SHIFT,
            ..
        } => Some(KeyAction::Last),
        KeyEvent {
            code: KeyCode::Home,
            ..
        } => Some(KeyAction::Root),
        KeyEvent {
            code: KeyCode::Char('.'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::ToggleHidden),
        KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::Quit),
        KeyEvent {
            code: KeyCode::Char('m'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::ToggleMarkdown),
        KeyEvent {
            code: KeyCode::Char('J'),
            modifiers: KeyModifiers::SHIFT,
            ..
        } => Some(KeyAction::PreviewLineDown),
        KeyEvent {
            code: KeyCode::Char('K'),
            modifiers: KeyModifiers::SHIFT,
            ..
        } => Some(KeyAction::PreviewLineUp),
        KeyEvent {
            code: KeyCode::Char('0'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::PreviewTop),
        KeyEvent {
            code: KeyCode::Char('$'),
            modifiers: KeyModifiers::SHIFT,
            ..
        } => Some(KeyAction::PreviewBottom),
        KeyEvent {
            code: KeyCode::Char(']'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::PreviewLinkNext),
        KeyEvent {
            code: KeyCode::Char('['),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::PreviewLinkPrevious),
        KeyEvent {
            code: KeyCode::Char('/'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::Search),
        KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::RefreshFile),
        KeyEvent {
            code: KeyCode::Char('R'),
            modifiers: KeyModifiers::SHIFT,
            ..
        } => Some(KeyAction::RefreshTree),
        KeyEvent {
            code: KeyCode::Char('e'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::OpenEditor),
        KeyEvent {
            code: KeyCode::Char('v'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::OpenInDevdeckEditor),
        KeyEvent {
            code: KeyCode::Char('o'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::OpenOs),
        KeyEvent {
            code: KeyCode::Char('y'),
            modifiers: KeyModifiers::NONE,
            ..
        } => Some(KeyAction::CopyRelative),
        KeyEvent {
            code: KeyCode::Char('Y'),
            modifiers: KeyModifiers::SHIFT,
            ..
        } => Some(KeyAction::CopyAbsolute),
        _ => None,
    }
}
