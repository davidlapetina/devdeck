use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use arboard::Clipboard;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    config::{self, ResolvedConfig, TerminalProfile},
    event::{EventSender, FsEventBatch},
    filesystem::tree::{FileTree, VisibleEntry},
    input::keymap::{map_key, KeyAction},
    preview::{self, PreviewContent, PreviewState},
    pty::input::key_event_to_bytes,
    search::SearchState,
    session::{
        launcher::{command_spec_from_profile, parent_shell_profile},
        CommandSpec, SessionId, SessionRegistry,
    },
    tabs::{ActivityState, Tab, TabContent, TabId, TerminalTabState},
};

const STATUS_TTL: Duration = Duration::from_secs(3);
const DEFAULT_TERMINAL_ROWS: u16 = 24;
const DEFAULT_TERMINAL_COLS: u16 = 80;
const RESTART_ON_EXIT_DELAY: Duration = Duration::from_secs(1);
const FAST_RESTART_WINDOW: Duration = Duration::from_secs(5);
const MAX_FAST_RESTARTS: u8 = 3;
const BACKGROUND_OUTPUT_QUIET_AFTER: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalOpen {
    Editor,
    OperatingSystem,
    Url(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Repository,
    Terminal,
    CommandPrefix,
    TabLauncher,
    RenameTab,
    ConfirmStop,
    ConfirmRestart,
    ConfirmQuit,
    Help,
    PromptOverlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalDimensions {
    pub rows: u16,
    pub cols: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherField {
    Source,
    Name,
    Command,
    Cwd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LauncherSource {
    Profile(usize),
    Shell,
    Custom,
}

#[derive(Debug, Clone)]
pub struct TabLauncherState {
    pub source_index: usize,
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub field: LauncherField,
}

impl Default for TabLauncherState {
    fn default() -> Self {
        Self {
            source_index: 0,
            name: String::new(),
            command: String::new(),
            cwd: String::new(),
            field: LauncherField::Source,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenameState {
    pub value: String,
}

#[derive(Debug, Clone, Default)]
pub struct PromptState {
    pub text: String,
}

impl Default for TerminalDimensions {
    fn default() -> Self {
        Self {
            rows: DEFAULT_TERMINAL_ROWS,
            cols: DEFAULT_TERMINAL_COLS,
        }
    }
}

pub struct App {
    pub root_path: PathBuf,
    pub tree: FileTree,
    pub visible_entries: Vec<VisibleEntry>,
    pub selected_path: Option<PathBuf>,
    pub selected_index: usize,
    pub expanded_directories: HashSet<PathBuf>,
    pub preview: PreviewState,
    pub preview_link_index: Option<usize>,
    pub search: SearchState,
    pub show_hidden: bool,
    pub watch_enabled: bool,
    pub markdown_rendered: bool,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub sessions: SessionRegistry,
    pub input_mode: InputMode,
    pub terminal_dimensions: TerminalDimensions,
    pub launcher: TabLauncherState,
    pub rename: RenameState,
    pub prompt: PromptState,
    pub status_message: Option<String>,
    pub should_quit: bool,
    next_tab_id: u64,
    status_set_at: Option<Instant>,
    confirm_tab: Option<usize>,
}

impl App {
    pub fn new(
        root_path: PathBuf,
        show_hidden: bool,
        watch_enabled: bool,
        config: ResolvedConfig,
    ) -> Result<Self> {
        let tree = FileTree::scan(&root_path, show_hidden)?;
        let root_path = tree.root.clone();
        let mut expanded_directories = HashSet::new();
        expanded_directories.insert(root_path.clone());
        let visible_entries = tree.visible_entries(&expanded_directories);
        let selected_path = visible_entries.first().map(|entry| entry.path.clone());
        let preview = selected_path
            .as_deref()
            .map(|path| PreviewState::load(path, false))
            .unwrap_or_default();

        let mut next_tab_id = 1;
        let mut tabs = vec![Tab::repository(TabId(next_tab_id))];
        next_tab_id += 1;
        for profile in config.tabs {
            tabs.push(Tab::terminal_tab(TabId(next_tab_id), profile, false));
            next_tab_id += 1;
        }

        let active_tab = config
            .workspace
            .default_tab
            .as_deref()
            .and_then(|default| tabs.iter().position(|tab| tab.title == default))
            .unwrap_or(0);
        let input_mode = base_mode_for_tab(&tabs[active_tab]);

        Ok(Self {
            root_path,
            tree,
            visible_entries,
            selected_path,
            selected_index: 0,
            expanded_directories,
            preview,
            preview_link_index: None,
            search: SearchState::default(),
            show_hidden,
            watch_enabled,
            markdown_rendered: true,
            tabs,
            active_tab,
            sessions: SessionRegistry::new(),
            input_mode,
            terminal_dimensions: TerminalDimensions::default(),
            launcher: TabLauncherState::default(),
            rename: RenameState::default(),
            prompt: PromptState::default(),
            status_message: None,
            should_quit: false,
            next_tab_id,
            status_set_at: None,
            confirm_tab: None,
        })
    }

    pub fn initialize_terminal_tabs(
        &mut self,
        event_tx: &EventSender,
        dimensions: TerminalDimensions,
    ) {
        self.terminal_dimensions = dimensions;
        let auto_start_tabs = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                tab.as_terminal()
                    .filter(|terminal| terminal.profile.auto_start)
                    .map(|_| index)
            })
            .collect::<Vec<_>>();

        for index in auto_start_tabs {
            self.start_terminal_tab(index, event_tx);
        }

        if self.active_tab != 0 {
            self.start_terminal_tab(self.active_tab, event_tx);
        }
    }

    pub fn handle_key(&mut self, event: KeyEvent, event_tx: &EventSender) -> Option<ExternalOpen> {
        match self.input_mode {
            InputMode::CommandPrefix => {
                self.handle_prefix_key(event, event_tx);
                return None;
            }
            InputMode::TabLauncher => {
                self.handle_launcher_key(event, event_tx);
                return None;
            }
            InputMode::RenameTab => {
                self.handle_rename_key(event);
                return None;
            }
            InputMode::ConfirmStop => {
                self.handle_confirm_stop_key(event);
                return None;
            }
            InputMode::ConfirmRestart => {
                self.handle_confirm_restart_key(event, event_tx);
                return None;
            }
            InputMode::ConfirmQuit => {
                self.handle_confirm_quit_key(event);
                return None;
            }
            InputMode::Help => {
                if matches!(
                    event.code,
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')
                ) {
                    self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
                }
                return None;
            }
            InputMode::PromptOverlay => {
                self.handle_prompt_key(event);
                return None;
            }
            InputMode::Repository | InputMode::Terminal => {}
        }

        if is_ctrl_b(event) {
            self.input_mode = InputMode::CommandPrefix;
            self.set_status(
                "COMMAND | 1..9 tab | n/p tab | c new | x stop | r restart | e reload | q quit | ? help",
            );
            return None;
        }

        if is_ctrl_g(event) {
            self.open_prompt_overlay();
            return None;
        }

        match self.active_content() {
            Some(TabContent::Repository) => self.handle_repository_key(event, event_tx),
            Some(TabContent::Terminal(_)) => {
                self.handle_terminal_key(event, event_tx);
                None
            }
            None => None,
        }
    }

    pub fn handle_filesystem_event(&mut self, batch: FsEventBatch) {
        let selected_before = self.selected_path.clone();
        if batch.tree_changed {
            self.reload_tree_preserving_selection();
        }

        if let Some(selected) = selected_before.as_deref() {
            let related = batch.paths.iter().any(|path| paths_related(selected, path));
            if related || batch.tree_changed {
                if selected.exists() {
                    let scroll = self.preview.scroll;
                    self.preview = PreviewState::load_with_scroll(selected, scroll);
                    self.preview_link_index = None;
                    self.set_status(format!("Refreshed: {}", self.relative_display(selected)));
                } else {
                    self.set_status("Selected file was deleted".to_string());
                }
            }
        }
    }

    pub fn handle_pty_output(&mut self, session_id: SessionId, bytes: &[u8]) {
        let now = Instant::now();
        self.sessions.handle_output(session_id, bytes);
        if bytes.is_empty() {
            return;
        }
        if let Some(index) = self.tab_index_for_session(session_id) {
            if index != self.active_tab {
                self.tabs[index].activity = ActivityState::OutputActive {
                    last_output_at: now,
                };
            }
        }
    }

    pub fn tick(&mut self, event_tx: &EventSender) {
        let now = Instant::now();
        if self
            .status_set_at
            .is_some_and(|set_at| now.duration_since(set_at) > STATUS_TTL)
        {
            self.status_message = None;
            self.status_set_at = None;
        }

        self.update_background_activity(now);

        for (session_id, exit_code) in self.sessions.poll_exits() {
            self.handle_terminal_exit(session_id, exit_code, event_tx);
        }

        let due_restarts = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                let terminal = tab.as_terminal()?;
                let due = terminal.pending_restart_at?;
                (due <= Instant::now()).then_some(index)
            })
            .collect::<Vec<_>>();

        for index in due_restarts {
            if let Some(session_id) = self.tabs[index]
                .as_terminal()
                .and_then(|terminal| terminal.session_id)
            {
                self.sessions.remove(session_id);
            }
            if let Some(terminal) = self.tabs[index].terminal_mut() {
                terminal.session_id = None;
                terminal.pending_restart_at = None;
            }
            self.start_terminal_tab(index, event_tx);
        }
    }

    fn update_background_activity(&mut self, now: Instant) {
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            if index == self.active_tab {
                continue;
            }
            let ActivityState::OutputActive { last_output_at } = tab.activity else {
                continue;
            };
            let Some(elapsed) = now.checked_duration_since(last_output_at) else {
                continue;
            };
            if elapsed >= BACKGROUND_OUTPUT_QUIET_AFTER {
                tab.activity = ActivityState::OutputQuiet;
            }
        }
    }

    fn handle_terminal_exit(
        &mut self,
        session_id: SessionId,
        exit_code: Option<i32>,
        event_tx: &EventSender,
    ) {
        let Some(index) = self.tab_index_for_session(session_id) else {
            return;
        };
        let should_return_to_files = self.tabs[index].return_to_files_on_exit;
        if let Some(terminal) = self.tabs[index].terminal_mut() {
            if terminal.profile.restart_on_exit {
                let fast_failure = terminal
                    .last_started_at
                    .is_some_and(|started| started.elapsed() < FAST_RESTART_WINDOW);
                terminal.restart_attempts = if fast_failure {
                    terminal.restart_attempts.saturating_add(1)
                } else {
                    0
                };
                if terminal.restart_attempts <= MAX_FAST_RESTARTS {
                    terminal.pending_restart_at = Some(Instant::now() + RESTART_ON_EXIT_DELAY);
                } else {
                    terminal.pending_restart_at = None;
                    terminal.state = TerminalTabState::Failed {
                        message: "restart_on_exit stopped after repeated fast exits".to_string(),
                    };
                    return;
                }
            }
            terminal.state = TerminalTabState::Exited { exit_code };
        }
        if index != self.active_tab {
            self.tabs[index].activity = ActivityState::OutputQuiet;
        }
        if index == self.active_tab && should_return_to_files {
            self.select_tab(0, event_tx);
            self.set_status("Editor exited");
        }
    }

    pub fn resize_active_terminal(&mut self, dimensions: TerminalDimensions) {
        self.terminal_dimensions = dimensions;
        let Some(session_id) = self.active_terminal_session_id() else {
            return;
        };
        if let Err(error) = self
            .sessions
            .resize(session_id, dimensions.rows, dimensions.cols)
        {
            self.set_status(format!("Resize failed: {error}"));
        }
    }

    pub fn selected_entry(&self) -> Option<&VisibleEntry> {
        self.visible_entries.get(self.selected_index)
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.selected_path.as_deref()
    }

    pub fn selected_external_path(&mut self) -> Option<PathBuf> {
        let path = self.selected_path.clone();
        if path.is_none() {
            self.set_status("No file selected".to_string());
        }
        path
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active_tab)
    }

    pub fn active_terminal_session_id(&self) -> Option<SessionId> {
        self.active_tab()
            .and_then(Tab::as_terminal)
            .and_then(|terminal| terminal.session_id)
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
        self.status_set_at = Some(Instant::now());
    }

    pub fn launcher_choice_labels(&self) -> Vec<String> {
        self.launcher_sources()
            .iter()
            .map(|source| self.launcher_source_label(*source))
            .collect()
    }

    pub fn relative_display(&self, path: &Path) -> String {
        path.strip_prefix(&self.root_path)
            .ok()
            .and_then(|relative| {
                if relative.as_os_str().is_empty() {
                    Some(".".to_string())
                } else {
                    Some(relative.to_string_lossy().to_string())
                }
            })
            .unwrap_or_else(|| path.to_string_lossy().to_string())
    }

    pub fn selected_working_directory(&self) -> PathBuf {
        let Some(selected) = self.selected_path.as_deref() else {
            return self.root_path.clone();
        };
        if selected.is_dir() {
            return selected.to_path_buf();
        }
        selected
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root_path.clone())
    }

    pub fn stop_all_sessions(&mut self) {
        self.sessions.stop_all();
    }

    fn handle_repository_key(
        &mut self,
        event: KeyEvent,
        event_tx: &EventSender,
    ) -> Option<ExternalOpen> {
        if self.search.active {
            self.handle_search_key(event);
            return None;
        }

        if self.handle_direct_app_key(event, event_tx) {
            return None;
        }

        let Some(action) = map_key(event) else {
            return None;
        };

        match action {
            KeyAction::MoveDown => self.move_selection(1),
            KeyAction::MoveUp => self.move_selection(-1),
            KeyAction::Activate if self.preview_link_index.is_some() => {
                return self.activate_preview_link();
            }
            KeyAction::ExpandOrEnter | KeyAction::Activate => self.expand_or_enter(),
            KeyAction::CollapseOrParent => self.collapse_or_parent(),
            KeyAction::First => self.select_index(0),
            KeyAction::Last => {
                if !self.visible_entries.is_empty() {
                    self.select_index(self.visible_entries.len() - 1);
                }
            }
            KeyAction::Root => self.go_root(),
            KeyAction::ToggleHidden => self.toggle_hidden(),
            KeyAction::Quit | KeyAction::CtrlC => self.request_quit_confirmation(),
            KeyAction::ToggleMarkdown => self.toggle_markdown(),
            KeyAction::PreviewPageDown => self.preview.scroll_page_down(),
            KeyAction::PreviewPageUp => self.preview.scroll_page_up(),
            KeyAction::PreviewLineDown => self.preview.scroll_lines(1),
            KeyAction::PreviewLineUp => self.preview.scroll_lines(-1),
            KeyAction::PreviewTop => self.preview.scroll_top(),
            KeyAction::PreviewBottom => self.preview.scroll_bottom(),
            KeyAction::PreviewLinkNext => self.focus_preview_link(1),
            KeyAction::PreviewLinkPrevious => self.focus_preview_link(-1),
            KeyAction::Search => {
                let entries = self.tree.all_entries();
                self.search.open(&entries);
            }
            KeyAction::RefreshFile => self.reload_selected_preview(true),
            KeyAction::RefreshTree => self.reload_tree_preserving_selection(),
            KeyAction::OpenEditor => return Some(ExternalOpen::Editor),
            KeyAction::OpenInDevdeckEditor => self.open_selected_file_in_terminal_editor(event_tx),
            KeyAction::OpenOs => return Some(ExternalOpen::OperatingSystem),
            KeyAction::CopyRelative => self.copy_selected_path(false),
            KeyAction::CopyAbsolute => self.copy_selected_path(true),
        }

        None
    }

    fn handle_terminal_key(&mut self, event: KeyEvent, event_tx: &EventSender) {
        let Some(terminal) = self.tabs.get(self.active_tab).and_then(Tab::as_terminal) else {
            return;
        };
        let state = terminal.state.clone();
        let session_id = terminal.session_id;

        match state {
            TerminalTabState::NotStarted => {
                if matches!(event.code, KeyCode::Enter) {
                    self.start_terminal_tab(self.active_tab, event_tx);
                } else {
                    self.handle_direct_app_key(event, event_tx);
                }
            }
            TerminalTabState::Exited { .. } | TerminalTabState::Failed { .. } => match event {
                KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }
                | KeyEvent {
                    code: KeyCode::Char('r'),
                    modifiers: KeyModifiers::NONE,
                    ..
                } => self.perform_restart(self.active_tab, event_tx),
                KeyEvent {
                    code: KeyCode::Char('x'),
                    modifiers: KeyModifiers::NONE,
                    ..
                } => self.perform_stop_or_close(self.active_tab),
                _ => {
                    self.handle_direct_app_key(event, event_tx);
                }
            },
            TerminalTabState::Running => {
                if let Some(session_id) = session_id {
                    if let Some(bytes) = key_event_to_bytes(event) {
                        if let Err(error) = self.sessions.write(session_id, &bytes) {
                            self.set_status(format!("Unable to write to terminal: {error}"));
                        }
                    }
                }
            }
            TerminalTabState::Starting => {}
        }
    }

    fn handle_prefix_key(&mut self, event: KeyEvent, event_tx: &EventSender) {
        self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);

        if matches!(event.code, KeyCode::Esc) {
            self.confirm_tab = None;
            self.set_status("Command cancelled");
            return;
        }
        if is_ctrl_b(event) {
            self.confirm_tab = None;
            if let Some(session_id) = self.active_terminal_session_id() {
                let _ = self.sessions.write(session_id, &[2]);
            }
            return;
        }

        match event {
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } if ('1'..='9').contains(&ch) => {
                self.confirm_tab = None;
                let index = ch as usize - '1' as usize;
                self.select_tab(index, event_tx);
            }
            KeyEvent {
                code: KeyCode::Char('f'),
                ..
            } => {
                self.confirm_tab = None;
                self.select_tab(0, event_tx);
            }
            KeyEvent {
                code: KeyCode::Char('n'),
                ..
            } => {
                self.confirm_tab = None;
                self.select_next_tab(event_tx);
            }
            KeyEvent {
                code: KeyCode::Char('p'),
                ..
            } => {
                self.confirm_tab = None;
                self.select_previous_tab(event_tx);
            }
            KeyEvent {
                code: KeyCode::Char('c'),
                ..
            } => self.open_tab_launcher(),
            KeyEvent {
                code: KeyCode::Char('x'),
                ..
            } => self.stop_or_close_active_terminal(),
            KeyEvent {
                code: KeyCode::Char('r'),
                ..
            } => self.restart_active_terminal(event_tx, false),
            KeyEvent {
                code: KeyCode::Char('e'),
                ..
            } => self.reload_configuration(),
            KeyEvent {
                code: KeyCode::Char('?'),
                ..
            } => {
                self.confirm_tab = None;
                self.input_mode = InputMode::Help;
            }
            KeyEvent {
                code: KeyCode::Char('q'),
                ..
            } => {
                self.confirm_tab = None;
                self.request_quit_confirmation();
            }
            KeyEvent {
                code: KeyCode::Char(','),
                ..
            } => self.open_rename_tab(),
            KeyEvent {
                code: KeyCode::Char('g'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.open_prompt_overlay(),
            KeyEvent {
                code: KeyCode::Char('G'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.open_prompt_overlay(),
            _ => {
                self.confirm_tab = None;
                self.set_status("Unknown command");
            }
        }
    }

    fn handle_launcher_key(&mut self, event: KeyEvent, event_tx: &EventSender) {
        match event {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
                self.set_status("New tab cancelled");
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            } => self.launcher.field = next_launcher_field(self.launcher.field),
            KeyEvent {
                code: KeyCode::BackTab,
                ..
            } => self.launcher.field = previous_launcher_field(self.launcher.field),
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::NONE,
                ..
            } if self.launcher.field == LauncherField::Source => self.adjust_launcher_source(1),
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
                ..
            } if self.launcher.field == LauncherField::Source => self.adjust_launcher_source(-1),
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                if let Some(field) = launcher_field_mut(&mut self.launcher) {
                    field.pop();
                }
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => self.submit_launcher(event_tx),
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } => {
                if let Some(field) = launcher_field_mut(&mut self.launcher) {
                    field.push(ch);
                }
            }
            _ => {}
        }
    }

    fn handle_rename_key(&mut self, event: KeyEvent) {
        match event {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
                self.set_status("Rename cancelled");
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.rename.value.pop();
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => self.submit_rename(),
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } => self.rename.value.push(ch),
            _ => {}
        }
    }

    fn handle_prompt_key(&mut self, event: KeyEvent) {
        match event {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
                self.prompt.text.clear();
                self.set_status("Prompt cancelled");
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.prompt.text.pop();
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::ALT) => self.prompt.text.push('\n'),
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => self.submit_prompt(),
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } => self.prompt.text.push(ch),
            _ => {}
        }
    }

    fn handle_confirm_stop_key(&mut self, event: KeyEvent) {
        match event {
            KeyEvent {
                code: KeyCode::Enter,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('y'),
                ..
            } => {
                if let Some(index) = self.confirm_tab.take() {
                    self.perform_stop_or_close(index);
                }
                if !matches!(self.input_mode, InputMode::Repository | InputMode::Terminal) {
                    self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
                }
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                ..
            } => {
                self.confirm_tab = None;
                self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
                self.set_status("Stop cancelled");
            }
            _ => {}
        }
    }

    fn handle_confirm_restart_key(&mut self, event: KeyEvent, event_tx: &EventSender) {
        match event {
            KeyEvent {
                code: KeyCode::Enter,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('y'),
                ..
            } => {
                if let Some(index) = self.confirm_tab.take() {
                    self.perform_restart(index, event_tx);
                }
                self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                ..
            } => {
                self.confirm_tab = None;
                self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
                self.set_status("Restart cancelled");
            }
            _ => {}
        }
    }

    fn handle_confirm_quit_key(&mut self, event: KeyEvent) {
        match event {
            KeyEvent {
                code: KeyCode::Enter,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('y'),
                ..
            } => {
                self.should_quit = true;
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                ..
            } => {
                self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
                self.set_status("Quit cancelled");
            }
            _ => {}
        }
    }

    fn handle_direct_app_key(&mut self, event: KeyEvent, event_tx: &EventSender) -> bool {
        match event {
            KeyEvent {
                code: KeyCode::Tab, ..
            } => {
                self.select_next_tab(event_tx);
                true
            }
            KeyEvent {
                code: KeyCode::BackTab,
                ..
            } => {
                self.select_previous_tab(event_tx);
                true
            }
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } if ('1'..='9').contains(&ch) => {
                let index = ch as usize - '1' as usize;
                self.select_tab(index, event_tx);
                true
            }
            KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.select_next_tab(event_tx);
                true
            }
            KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.select_previous_tab(event_tx);
                true
            }
            KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.open_tab_launcher();
                true
            }
            KeyEvent {
                code: KeyCode::Char('?'),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } => {
                self.input_mode = InputMode::Help;
                true
            }
            KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.request_quit_confirmation();
                true
            }
            _ => false,
        }
    }

    fn request_quit_confirmation(&mut self) {
        self.confirm_tab = None;
        self.input_mode = InputMode::ConfirmQuit;
        self.set_status("Quit DevDeck? Enter/y confirm, Esc/n cancel");
    }

    fn select_tab(&mut self, index: usize, event_tx: &EventSender) {
        if index >= self.tabs.len() {
            return;
        }
        let previous = self.active_tab;
        if previous != index {
            self.watch_recent_output_for_background_tab(previous, Instant::now());
        }
        self.active_tab = index;
        self.tabs[index].activity = ActivityState::None;
        self.input_mode = base_mode_for_tab(&self.tabs[index]);
        self.confirm_tab = None;
        self.start_terminal_tab(index, event_tx);
        self.resize_active_terminal(self.terminal_dimensions);
    }

    fn watch_recent_output_for_background_tab(&mut self, index: usize, now: Instant) {
        let Some(last_output_at) = self
            .tabs
            .get(index)
            .and_then(Tab::as_terminal)
            .filter(|terminal| terminal.state.is_running())
            .and_then(|terminal| terminal.session_id)
            .and_then(|session_id| self.sessions.session(session_id))
            .and_then(|session| session.last_activity)
        else {
            return;
        };
        let Some(elapsed) = now.checked_duration_since(last_output_at) else {
            return;
        };
        if elapsed < BACKGROUND_OUTPUT_QUIET_AFTER {
            self.tabs[index].activity = ActivityState::OutputActive { last_output_at };
        }
    }

    fn select_next_tab(&mut self, event_tx: &EventSender) {
        if self.tabs.is_empty() {
            return;
        }
        let next = (self.active_tab + 1) % self.tabs.len();
        self.select_tab(next, event_tx);
    }

    fn select_previous_tab(&mut self, event_tx: &EventSender) {
        if self.tabs.is_empty() {
            return;
        }
        let previous = if self.active_tab == 0 {
            self.tabs.len() - 1
        } else {
            self.active_tab - 1
        };
        self.select_tab(previous, event_tx);
    }

    fn open_tab_launcher(&mut self) {
        self.confirm_tab = None;
        self.launcher = TabLauncherState {
            cwd: self.relative_display(&self.selected_working_directory()),
            ..Default::default()
        };
        self.apply_launcher_source();
        self.input_mode = InputMode::TabLauncher;
        self.set_status("New tab: choose source, Tab fields, Enter launch, Esc cancel");
    }

    fn submit_launcher(&mut self, event_tx: &EventSender) {
        let name = self.launcher.name.trim().to_string();
        let command_line = self.launcher.command.trim().to_string();
        if name.is_empty() {
            self.set_status("Tab name cannot be empty");
            self.launcher.field = LauncherField::Name;
            return;
        }
        if command_line.is_empty() {
            self.set_status("Command cannot be empty");
            self.launcher.field = LauncherField::Command;
            return;
        }
        if self.tabs.iter().any(|tab| tab.title == name) {
            self.set_status(format!("Tab name already exists: {name}"));
            self.launcher.field = LauncherField::Name;
            return;
        }

        let mut parts = match shell_words::split(&command_line) {
            Ok(parts) if !parts.is_empty() => parts,
            Ok(_) => {
                self.set_status("Command cannot be empty");
                return;
            }
            Err(error) => {
                self.set_status(format!("Invalid command line: {error}"));
                return;
            }
        };
        let command = parts.remove(0);
        let cwd = if self.launcher.cwd.trim().is_empty() {
            Some(self.root_path.clone())
        } else {
            let cwd = PathBuf::from(self.launcher.cwd.trim());
            let cwd = if cwd.is_absolute() {
                cwd
            } else {
                self.root_path.join(cwd)
            };
            if !cwd.is_dir() {
                self.set_status("Working directory does not exist");
                self.launcher.field = LauncherField::Cwd;
                return;
            }
            Some(cwd)
        };

        let mut profile = self.launcher_profile_template();
        profile.name = name;
        profile.command = command;
        profile.args = parts;
        profile.cwd = cwd;
        profile.auto_start = true;
        let index = self.tabs.len();
        self.tabs
            .push(Tab::terminal_tab(TabId(self.next_tab_id), profile, true));
        self.next_tab_id += 1;
        self.input_mode = InputMode::Terminal;
        self.select_tab(index, event_tx);
    }

    fn launcher_sources(&self) -> Vec<LauncherSource> {
        let mut sources = self
            .tabs
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(index, tab)| {
                (!tab.temporary && tab.as_terminal().is_some())
                    .then_some(LauncherSource::Profile(index))
            })
            .collect::<Vec<_>>();
        sources.push(LauncherSource::Shell);
        sources.push(LauncherSource::Custom);
        sources
    }

    fn launcher_source_label(&self, source: LauncherSource) -> String {
        match source {
            LauncherSource::Profile(index) => self
                .tabs
                .get(index)
                .and_then(Tab::as_terminal)
                .map(|terminal| {
                    format!(
                        "{} - {}",
                        terminal.profile.name,
                        command_line_for_profile(&terminal.profile)
                    )
                })
                .unwrap_or_else(|| "Configured command".to_string()),
            LauncherSource::Shell => {
                let profile = parent_shell_profile(&self.selected_working_directory());
                format!("New shell - {}", command_line_for_profile(&profile))
            }
            LauncherSource::Custom => "Custom command".to_string(),
        }
    }

    fn selected_launcher_source(&self) -> LauncherSource {
        let sources = self.launcher_sources();
        sources
            .get(self.launcher.source_index)
            .copied()
            .unwrap_or(LauncherSource::Custom)
    }

    fn adjust_launcher_source(&mut self, delta: isize) {
        let count = self.launcher_sources().len();
        if count == 0 {
            return;
        }
        let current = self.launcher.source_index.min(count - 1);
        self.launcher.source_index = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize).min(count - 1)
        };
        self.apply_launcher_source();
    }

    fn apply_launcher_source(&mut self) {
        let source = self.selected_launcher_source();
        let default_cwd = self.relative_display(&self.selected_working_directory());
        match source {
            LauncherSource::Profile(index) => {
                if let Some(profile) = self
                    .tabs
                    .get(index)
                    .and_then(Tab::as_terminal)
                    .map(|terminal| terminal.profile.clone())
                {
                    self.launcher.name = self.unique_temporary_title(&profile.name);
                    self.launcher.command = command_line_for_profile(&profile);
                    self.launcher.cwd = profile
                        .cwd
                        .as_deref()
                        .map(|cwd| self.relative_display(cwd))
                        .unwrap_or(default_cwd);
                }
            }
            LauncherSource::Shell => {
                let profile = parent_shell_profile(&self.selected_working_directory());
                self.launcher.name = self.unique_temporary_title(&profile.name);
                self.launcher.command = command_line_for_profile(&profile);
                self.launcher.cwd = profile
                    .cwd
                    .as_deref()
                    .map(|cwd| self.relative_display(cwd))
                    .unwrap_or(default_cwd);
            }
            LauncherSource::Custom => {
                self.launcher.name.clear();
                self.launcher.command.clear();
                self.launcher.cwd = default_cwd;
            }
        }
    }

    fn launcher_profile_template(&self) -> TerminalProfile {
        match self.selected_launcher_source() {
            LauncherSource::Profile(index) => self
                .tabs
                .get(index)
                .and_then(Tab::as_terminal)
                .map(|terminal| terminal.profile.clone())
                .unwrap_or_else(|| custom_launcher_profile(&self.root_path)),
            LauncherSource::Shell => parent_shell_profile(&self.selected_working_directory()),
            LauncherSource::Custom => custom_launcher_profile(&self.root_path),
        }
    }

    fn open_rename_tab(&mut self) {
        if self.active_tab == 0 {
            self.set_status("Files tab cannot be renamed");
            return;
        }
        if !self.tabs[self.active_tab].temporary {
            self.set_status("Only temporary tabs can be renamed");
            return;
        }
        self.rename.value = self.tabs[self.active_tab].title.clone();
        self.input_mode = InputMode::RenameTab;
        self.set_status("Rename tab: Enter save, Esc cancel");
    }

    fn submit_rename(&mut self) {
        let name = self.rename.value.trim().to_string();
        if name.is_empty() {
            self.set_status("Tab name cannot be empty");
            return;
        }
        if self
            .tabs
            .iter()
            .enumerate()
            .any(|(index, tab)| index != self.active_tab && tab.title == name)
        {
            self.set_status(format!("Tab name already exists: {name}"));
            return;
        }
        self.tabs[self.active_tab].title = name.clone();
        if let Some(terminal) = self.tabs[self.active_tab].terminal_mut() {
            terminal.profile.name = name;
        }
        self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
        self.set_status("Tab renamed");
    }

    fn open_prompt_overlay(&mut self) {
        let Some(terminal) = self.active_tab().and_then(Tab::as_terminal) else {
            self.set_status("No active terminal session");
            return;
        };
        if !matches!(terminal.state, TerminalTabState::Running) {
            self.set_status("Active terminal is not running");
            return;
        }
        self.prompt.text.clear();
        self.input_mode = InputMode::PromptOverlay;
        self.set_status("Prompt overlay: Enter send, Alt-Enter newline, Esc cancel");
    }

    fn submit_prompt(&mut self) {
        let Some(session_id) = self.active_terminal_session_id() else {
            self.set_status("No active terminal session");
            self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
            return;
        };
        let mut bytes = self.prompt.text.as_bytes().to_vec();
        bytes.push(b'\n');
        match self.sessions.write(session_id, &bytes) {
            Ok(()) => {
                self.prompt.text.clear();
                self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
                self.set_status("Prompt sent");
            }
            Err(error) => self.set_status(format!("Unable to send prompt: {error}")),
        }
    }

    fn restart_active_terminal(&mut self, event_tx: &EventSender, force: bool) {
        if self.active_tab == 0 {
            self.set_status("Files tab cannot be restarted");
            return;
        }
        let index = self.active_tab;
        let is_running = self.tabs[index]
            .as_terminal()
            .is_some_and(|terminal| terminal.state.is_running());
        if is_running && !force {
            self.confirm_tab = Some(index);
            self.input_mode = InputMode::ConfirmRestart;
            self.set_status("Restart this terminal? Enter/y confirm, Esc/n cancel");
            return;
        }
        self.perform_restart(index, event_tx);
    }

    fn perform_restart(&mut self, index: usize, event_tx: &EventSender) {
        if index == 0 || index >= self.tabs.len() {
            return;
        }
        if let Some(session_id) = self.tabs[index]
            .as_terminal()
            .and_then(|terminal| terminal.session_id)
        {
            let _ = self.sessions.stop(session_id);
            self.sessions.remove(session_id);
        }
        if let Some(terminal) = self.tabs[index].terminal_mut() {
            terminal.session_id = None;
            terminal.pending_restart_at = None;
            terminal.restart_attempts = 0;
            terminal.state = TerminalTabState::NotStarted;
            terminal.requires_restart = false;
        }
        self.start_terminal_tab(index, event_tx);
        self.set_status("Terminal restarted");
    }

    fn reload_configuration(&mut self) {
        let active_title = self.tabs.get(self.active_tab).map(|tab| tab.title.clone());
        match config::load_config(&self.root_path) {
            Ok(config) => {
                let (added, changed, removed) = self.reconcile_config(config);
                if let Some(title) = active_title {
                    if let Some(index) = self.tabs.iter().position(|tab| tab.title == title) {
                        self.active_tab = index;
                    } else {
                        self.active_tab = 0;
                    }
                    self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
                }
                self.set_status(format!(
                    "Configuration reloaded: {added} added, {changed} changed, {removed} removed"
                ));
            }
            Err(error) => self.set_status(format!("Configuration reload failed: {error:#}")),
        }
    }

    fn reconcile_config(&mut self, config: ResolvedConfig) -> (usize, usize, usize) {
        let mut added = 0;
        let mut changed = 0;
        let mut removed = 0;
        let new_names = config
            .tabs
            .iter()
            .map(|profile| profile.name.clone())
            .collect::<HashSet<_>>();

        for profile in config.tabs {
            if let Some(index) = self
                .tabs
                .iter()
                .position(|tab| !tab.temporary && tab.title == profile.name)
            {
                if let Some(terminal) = self.tabs[index].terminal_mut() {
                    if terminal.profile != profile {
                        changed += 1;
                        let running = terminal.state.is_running();
                        terminal.profile = profile;
                        terminal.removed_from_config = false;
                        if running {
                            terminal.requires_restart = true;
                        } else {
                            terminal.requires_restart = false;
                        }
                    }
                }
            } else {
                self.tabs
                    .push(Tab::terminal_tab(TabId(self.next_tab_id), profile, false));
                self.next_tab_id += 1;
                added += 1;
            }
        }

        let mut index = 1;
        while index < self.tabs.len() {
            let remove = if self.tabs[index].temporary {
                false
            } else if let Some(terminal) = self.tabs[index].as_terminal() {
                !new_names.contains(&terminal.profile.name)
            } else {
                false
            };
            if remove {
                let running = self.tabs[index]
                    .as_terminal()
                    .is_some_and(|terminal| terminal.state.is_running());
                if running {
                    if let Some(terminal) = self.tabs[index].terminal_mut() {
                        terminal.removed_from_config = true;
                        terminal.requires_restart = true;
                    }
                    removed += 1;
                    index += 1;
                } else {
                    self.tabs.remove(index);
                    removed += 1;
                    if self.active_tab >= index {
                        self.active_tab = self.active_tab.saturating_sub(1);
                    }
                }
            } else {
                index += 1;
            }
        }

        (added, changed, removed)
    }

    fn start_terminal_tab(&mut self, index: usize, event_tx: &EventSender) {
        let Some((old_session_id, title, profile)) = self.tabs.get(index).and_then(|tab| {
            let terminal = tab.as_terminal()?;
            if terminal.state.is_running() {
                return None;
            }
            Some((
                terminal.session_id,
                tab.title.clone(),
                terminal.profile.clone(),
            ))
        }) else {
            return;
        };

        if let Some(session_id) = old_session_id {
            self.sessions.remove(session_id);
            if let Some(terminal) = self.tabs[index].terminal_mut() {
                terminal.session_id = None;
            }
        }

        self.tabs[index].activity = ActivityState::None;
        if let Some(terminal) = self.tabs[index].terminal_mut() {
            terminal.state = TerminalTabState::Starting;
        }

        let command = self.command_spec_for_profile(&profile);
        match self.sessions.start(
            title,
            command.clone(),
            self.terminal_dimensions.rows,
            self.terminal_dimensions.cols,
            event_tx.clone(),
        ) {
            Ok(session_id) => {
                if let Some(terminal) = self.tabs[index].terminal_mut() {
                    terminal.session_id = Some(session_id);
                    terminal.state = TerminalTabState::Running;
                    terminal.last_started_at = Some(Instant::now());
                    terminal.pending_restart_at = None;
                }
            }
            Err(error) => {
                if let Some(terminal) = self.tabs[index].terminal_mut() {
                    terminal.session_id = None;
                    terminal.pending_restart_at = None;
                    terminal.state = TerminalTabState::Failed {
                        message: spawn_failure_message(&command, &error),
                    };
                }
                self.set_status(format!(
                    "Unable to start terminal: {}",
                    spawn_failure_message(&command, &error)
                ));
            }
        }
    }

    fn command_spec_for_profile(&self, profile: &TerminalProfile) -> CommandSpec {
        let default_cwd = if launches_from_selected_directory(profile) {
            self.selected_working_directory()
        } else {
            self.root_path.clone()
        };
        command_spec_from_profile(profile, &default_cwd)
    }

    fn stop_or_close_active_terminal(&mut self) {
        if self.active_tab == 0 {
            self.set_status("Files tab cannot be closed");
            return;
        }

        let index = self.active_tab;
        let is_running = self.tabs[index]
            .as_terminal()
            .is_some_and(|terminal| terminal.state.is_running());

        if is_running {
            self.confirm_tab = Some(index);
            self.input_mode = InputMode::ConfirmStop;
            self.set_status("Stop this terminal? Enter/y confirm, Esc/n cancel");
            return;
        }

        self.perform_stop_or_close(index);
    }

    fn perform_stop_or_close(&mut self, index: usize) {
        if index == 0 || index >= self.tabs.len() {
            return;
        }

        let session_id = self.tabs[index]
            .as_terminal()
            .and_then(|terminal| terminal.session_id);
        if let Some(session_id) = session_id {
            let _ = self.sessions.stop(session_id);
            self.sessions.remove(session_id);
        }

        if self.tabs[index].temporary {
            self.tabs.remove(index);
            self.active_tab = index
                .saturating_sub(1)
                .min(self.tabs.len().saturating_sub(1));
            self.input_mode = base_mode_for_tab(&self.tabs[self.active_tab]);
            self.set_status("Temporary tab closed");
        } else if let Some(terminal) = self.tabs[index].terminal_mut() {
            terminal.session_id = None;
            terminal.pending_restart_at = None;
            terminal.state = TerminalTabState::NotStarted;
            self.set_status("Terminal stopped");
        }
    }

    fn open_selected_file_in_terminal_editor(&mut self, event_tx: &EventSender) {
        let Some(path) = self.selected_path.clone() else {
            self.set_status("No file selected");
            return;
        };
        if path.is_dir() {
            self.set_status("Cannot open a directory in the editor");
            return;
        }

        let absolute_path = match path.canonicalize() {
            Ok(path) => path,
            Err(error) => {
                self.set_status(format!("Unable to resolve selected file: {error}"));
                return;
            }
        };

        let (command, mut args) = match resolve_editor_command() {
            Ok(command) => command,
            Err(message) => {
                self.set_status(message);
                return;
            }
        };
        args.push(absolute_path.to_string_lossy().to_string());

        let base_title = absolute_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("editor")
            .to_string();
        let title = self.unique_temporary_title(&base_title);
        let profile = TerminalProfile {
            name: title,
            command,
            args,
            cwd: Some(self.root_path.clone()),
            env: Default::default(),
            auto_start: true,
            restart_on_exit: false,
        };

        let index = self.tabs.len();
        self.tabs
            .push(Tab::terminal_tab(TabId(self.next_tab_id), profile, true));
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.return_to_files_on_exit = true;
        }
        self.next_tab_id += 1;
        self.select_tab(index, event_tx);
    }

    fn unique_temporary_title(&self, base: &str) -> String {
        if !self.tabs.iter().any(|tab| tab.title == base) {
            return base.to_string();
        }

        for suffix in 2.. {
            let candidate = format!("{base} ({suffix})");
            if !self.tabs.iter().any(|tab| tab.title == candidate) {
                return candidate;
            }
        }
        unreachable!()
    }

    fn tab_index_for_session(&self, session_id: SessionId) -> Option<usize> {
        self.tabs.iter().position(|tab| {
            tab.as_terminal()
                .and_then(|terminal| terminal.session_id)
                .is_some_and(|id| id == session_id)
        })
    }

    fn active_content(&self) -> Option<&TabContent> {
        self.tabs.get(self.active_tab).map(|tab| &tab.content)
    }

    fn handle_search_key(&mut self, event: KeyEvent) {
        let entries = self.tree.all_entries();
        match event {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => self.search.cancel(),
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                if let Some(path) = self.search.selected_path().map(Path::to_path_buf) {
                    self.search.cancel();
                    self.select_path(&path, true);
                }
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => self.search.backspace(&entries),
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::NONE,
                ..
            } => self.search.move_down(),
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
                ..
            } => self.search.move_up(),
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } => self.search.push(ch, &entries),
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.visible_entries.is_empty() {
            return;
        }

        let next = if delta.is_negative() {
            self.selected_index.saturating_sub(delta.unsigned_abs())
        } else {
            self.selected_index.saturating_add(delta as usize)
        }
        .min(self.visible_entries.len() - 1);

        self.select_index(next);
    }

    fn select_index(&mut self, index: usize) {
        if let Some(entry) = self.visible_entries.get(index) {
            let path = entry.path.clone();
            self.selected_index = index;
            self.selected_path = Some(path.clone());
            self.preview = PreviewState::load(&path, false);
            self.preview_link_index = None;
        }
    }

    fn select_path(&mut self, path: &Path, reset_preview: bool) {
        self.expand_ancestors(path);
        self.refresh_visible_entries();

        let target = if self.tree.contains_path(path) {
            path.to_path_buf()
        } else {
            self.tree.nearest_existing_ancestor(path)
        };

        if let Some(index) = self
            .visible_entries
            .iter()
            .position(|entry| entry.path == target)
        {
            self.selected_index = index;
            self.selected_path = Some(target.clone());
            if reset_preview {
                self.preview = PreviewState::load(&target, false);
                self.preview_link_index = None;
            }
        }
    }

    fn expand_or_enter(&mut self) {
        let Some(entry) = self.selected_entry().cloned() else {
            return;
        };
        if !entry.is_dir {
            return;
        }

        let was_expanded = self.expanded_directories.contains(&entry.path);
        self.expanded_directories.insert(entry.path.clone());
        self.refresh_visible_entries();

        if !was_expanded {
            self.select_path(&entry.path, false);
        } else if let Some(index) = self
            .visible_entries
            .iter()
            .position(|visible| visible.path == entry.path)
        {
            let child_index = (index + 1).min(self.visible_entries.len().saturating_sub(1));
            self.select_index(child_index);
        }
    }

    fn collapse_or_parent(&mut self) {
        let Some(entry) = self.selected_entry().cloned() else {
            return;
        };

        if entry.is_dir
            && self.expanded_directories.contains(&entry.path)
            && entry.path != self.root_path
        {
            self.expanded_directories.remove(&entry.path);
            self.refresh_visible_entries();
            self.select_path(&entry.path, false);
            return;
        }

        if let Some(parent) = entry.path.parent().map(Path::to_path_buf) {
            if parent.starts_with(&self.root_path) {
                self.select_path(&parent, true);
            }
        }
    }

    fn go_root(&mut self) {
        self.expanded_directories.insert(self.root_path.clone());
        let root = self.root_path.clone();
        self.select_path(&root, true);
    }

    fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.reload_tree_preserving_selection();
        self.set_status(if self.show_hidden {
            "Hidden files shown"
        } else {
            "Hidden files hidden"
        });
    }

    fn toggle_markdown(&mut self) {
        self.markdown_rendered = !self.markdown_rendered;
        if !self.markdown_rendered {
            self.preview_link_index = None;
        }
        self.set_status(if self.markdown_rendered {
            "Markdown rendered"
        } else {
            "Markdown raw"
        });
    }

    fn preview_links(&self) -> Vec<preview::markdown::MarkdownLink> {
        if !self.markdown_rendered {
            return Vec::new();
        }

        match &self.preview.content {
            PreviewContent::Markdown { content } => preview::markdown::collect_links(content),
            _ => Vec::new(),
        }
    }

    fn focus_preview_link(&mut self, delta: isize) {
        let links = self.preview_links();
        if links.is_empty() {
            self.preview_link_index = None;
            self.set_status("No links in rendered Markdown preview");
            return;
        }

        let len = links.len();
        let next = match self.preview_link_index {
            Some(index) if delta.is_negative() => {
                if index == 0 {
                    len - 1
                } else {
                    index - 1
                }
            }
            Some(index) => (index + 1) % len,
            None if delta.is_negative() => len - 1,
            None => 0,
        };
        self.preview_link_index = Some(next);
        self.scroll_to_preview_link(next);
        self.set_status(format!("Link {}/{}: {}", next + 1, len, links[next].target));
    }

    fn activate_preview_link(&mut self) -> Option<ExternalOpen> {
        let Some(index) = self.preview_link_index else {
            return None;
        };
        let links = self.preview_links();
        let Some(link) = links.get(index) else {
            self.preview_link_index = None;
            self.set_status("Link is no longer available");
            return None;
        };
        let target = link.target.trim().to_string();
        if target.is_empty() {
            self.set_status("Link target is empty");
            return None;
        }
        if is_external_link_target(&target) {
            return Some(ExternalOpen::Url(target));
        }

        let (path_part, fragment) = split_link_fragment(&target);
        let Some(path) = self.resolve_markdown_link_path(path_part) else {
            self.set_status(format!("Link target not found: {target}"));
            return None;
        };

        if !path.exists() {
            self.set_status(format!("Link target not found: {target}"));
            return None;
        }

        self.select_path(&path, true);
        if let Some(fragment) = fragment {
            self.scroll_to_markdown_anchor(fragment);
        }
        self.set_status(format!("Opened link: {}", self.relative_display(&path)));
        None
    }

    fn scroll_to_preview_link(&mut self, link_index: usize) {
        let PreviewContent::Markdown { content } = &self.preview.content else {
            return;
        };
        let width = self.preview_render_width();
        if width == 0 {
            return;
        }
        if let Some(line) = preview::markdown::focused_link_line(content, width, link_index) {
            self.scroll_preview_to_line(line);
        }
    }

    fn scroll_to_markdown_anchor(&mut self, fragment: &str) {
        let PreviewContent::Markdown { content } = &self.preview.content else {
            return;
        };
        let width = self.preview_render_width();
        if width == 0 {
            return;
        }
        if let Some(line) = preview::markdown::anchor_line(content, width, fragment) {
            self.scroll_preview_to_line(line);
        }
    }

    fn scroll_preview_to_line(&mut self, line: usize) {
        if line < self.preview.scroll {
            self.preview.scroll = line;
        } else {
            let visible_bottom = self
                .preview
                .scroll
                .saturating_add(self.preview.viewport_height.saturating_sub(1));
            if line > visible_bottom {
                self.preview.scroll = line.saturating_sub(self.preview.viewport_height / 2);
            }
        }
        self.preview.clamp_scroll();
    }

    fn preview_render_width(&self) -> usize {
        self.preview.viewport_width.saturating_sub(2).max(1)
    }

    fn resolve_markdown_link_path(&self, path_part: &str) -> Option<PathBuf> {
        let current = self
            .preview
            .path
            .as_deref()
            .or(self.selected_path.as_deref())?;
        if path_part.is_empty() {
            return Some(current.to_path_buf());
        }

        let raw_path = Path::new(path_part);
        let candidate = if raw_path.is_absolute() {
            if raw_path.starts_with(&self.root_path) {
                raw_path.to_path_buf()
            } else {
                self.root_path.join(path_part.trim_start_matches('/'))
            }
        } else {
            current
                .parent()
                .unwrap_or(self.root_path.as_path())
                .join(raw_path)
        };

        Some(candidate.canonicalize().unwrap_or(candidate))
    }

    fn copy_selected_path(&mut self, absolute: bool) {
        let Some(path) = self.selected_path.clone() else {
            self.set_status("No file selected");
            return;
        };

        let value = if absolute {
            path.to_string_lossy().to_string()
        } else {
            self.relative_display(&path)
        };

        match Clipboard::new().and_then(|mut clipboard| clipboard.set_text(value.clone())) {
            Ok(()) => self.set_status(format!("Copied: {value}")),
            Err(error) => self.set_status(format!("Clipboard unavailable: {error}")),
        }
    }

    fn reload_selected_preview(&mut self, preserve_scroll: bool) {
        let Some(path) = self.selected_path.clone() else {
            return;
        };
        let scroll = if preserve_scroll {
            self.preview.scroll
        } else {
            0
        };
        self.preview = PreviewState::load_with_scroll(&path, scroll);
        self.preview_link_index = None;
        self.set_status(format!("Reloaded: {}", self.relative_display(&path)));
    }

    fn reload_tree_preserving_selection(&mut self) {
        let previous = self
            .selected_path
            .clone()
            .unwrap_or_else(|| self.root_path.clone());
        match FileTree::scan(&self.root_path, self.show_hidden) {
            Ok(tree) => {
                self.tree = tree;
                self.expanded_directories.insert(self.root_path.clone());
                self.expanded_directories = self
                    .expanded_directories
                    .iter()
                    .filter(|path| self.tree.contains_path(path))
                    .cloned()
                    .collect();
                self.select_path(&previous, true);
                self.search.update(&self.tree.all_entries());
            }
            Err(error) => self.set_status(format!("Refresh failed: {error}")),
        }
    }

    fn expand_ancestors(&mut self, path: &Path) {
        let mut current = path.parent();
        while let Some(parent) = current {
            if parent.starts_with(&self.root_path) {
                self.expanded_directories.insert(parent.to_path_buf());
            }
            if parent == self.root_path {
                break;
            }
            current = parent.parent();
        }
    }

    fn refresh_visible_entries(&mut self) {
        self.visible_entries = self.tree.visible_entries(&self.expanded_directories);
        if self.selected_index >= self.visible_entries.len() {
            self.selected_index = self.visible_entries.len().saturating_sub(1);
        }
        self.selected_path = self
            .visible_entries
            .get(self.selected_index)
            .map(|entry| entry.path.clone());
    }
}

fn resolve_editor_command() -> Result<(String, Vec<String>), String> {
    let raw = env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_string());
    let mut parts = shell_words::split(&raw)
        .map_err(|error| format!("Unable to launch editor: invalid editor command: {error}"))?;
    if parts.is_empty() {
        return Err("Unable to launch editor: executable not found".to_string());
    }
    let executable = parts.remove(0);
    if !executable_exists(&executable) {
        return Err("Unable to launch editor: executable not found".to_string());
    }
    Ok((executable, parts))
}

fn executable_exists(executable: &str) -> bool {
    let path = Path::new(executable);
    if path.components().count() > 1 {
        return path.is_file();
    }

    env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|directory| directory.join(executable).is_file())
    })
}

fn spawn_failure_message(command: &CommandSpec, error: &anyhow::Error) -> String {
    let causes = error
        .chain()
        .map(|cause| cause.to_string())
        .collect::<Vec<_>>();
    let combined = causes.join("\n").to_ascii_lowercase();
    if combined.contains("no such file")
        || combined.contains("not found")
        || combined.contains("could not find")
        || !executable_exists(&command.executable)
    {
        format!("Executable not found: {}", command.executable)
    } else {
        let detail = causes
            .iter()
            .rev()
            .find(|cause| !cause.starts_with("Unable to start session"))
            .or_else(|| causes.last())
            .cloned()
            .unwrap_or_else(|| "unknown error".to_string());
        format!("{}: {detail}", command.executable)
    }
}

fn paths_related(selected: &Path, changed: &Path) -> bool {
    selected == changed || changed.starts_with(selected) || selected.starts_with(changed)
}

fn launches_from_selected_directory(profile: &TerminalProfile) -> bool {
    if profile.cwd.is_some() {
        return false;
    }
    is_codex_or_claude(&profile.name) || is_codex_or_claude(command_basename(&profile.command))
}

fn command_basename(command: &str) -> &str {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
}

fn is_codex_or_claude(value: &str) -> bool {
    value.eq_ignore_ascii_case("codex") || value.eq_ignore_ascii_case("claude")
}

fn command_line_for_profile(profile: &TerminalProfile) -> String {
    let mut words = Vec::with_capacity(profile.args.len() + 1);
    words.push(profile.command.as_str());
    words.extend(profile.args.iter().map(String::as_str));
    shell_words::join(words)
}

fn custom_launcher_profile(root_path: &Path) -> TerminalProfile {
    TerminalProfile {
        name: String::new(),
        command: String::new(),
        args: Vec::new(),
        cwd: Some(root_path.to_path_buf()),
        env: Default::default(),
        auto_start: true,
        restart_on_exit: false,
    }
}

fn split_link_fragment(target: &str) -> (&str, Option<&str>) {
    target
        .split_once('#')
        .map(|(path, fragment)| (path, Some(fragment)))
        .unwrap_or((target, None))
}

fn is_external_link_target(target: &str) -> bool {
    let Some((scheme, _)) = target.split_once(':') else {
        return false;
    };
    scheme
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

fn is_ctrl_b(event: KeyEvent) -> bool {
    matches!(
        event,
        KeyEvent {
            code: KeyCode::Char('b'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
    )
}

fn is_ctrl_g(event: KeyEvent) -> bool {
    matches!(
        event,
        KeyEvent {
            code: KeyCode::Char('g'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
    )
}

fn next_launcher_field(field: LauncherField) -> LauncherField {
    match field {
        LauncherField::Source => LauncherField::Name,
        LauncherField::Name => LauncherField::Command,
        LauncherField::Command => LauncherField::Cwd,
        LauncherField::Cwd => LauncherField::Source,
    }
}

fn previous_launcher_field(field: LauncherField) -> LauncherField {
    match field {
        LauncherField::Source => LauncherField::Cwd,
        LauncherField::Name => LauncherField::Source,
        LauncherField::Command => LauncherField::Name,
        LauncherField::Cwd => LauncherField::Command,
    }
}

fn launcher_field_mut(launcher: &mut TabLauncherState) -> Option<&mut String> {
    match launcher.field {
        LauncherField::Source => None,
        LauncherField::Name => Some(&mut launcher.name),
        LauncherField::Command => Some(&mut launcher.command),
        LauncherField::Cwd => Some(&mut launcher.cwd),
    }
}

fn base_mode_for_tab(tab: &Tab) -> InputMode {
    match tab.content {
        TabContent::Repository => InputMode::Repository,
        TabContent::Terminal(_) => InputMode::Terminal,
    }
}

pub fn validate_root(path: PathBuf) -> Result<PathBuf> {
    let root = path
        .canonicalize()
        .with_context(|| format!("Path does not exist: {}", path.display()))?;
    if !root.is_dir() {
        anyhow::bail!("Path is not a directory: {}", root.display());
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, time::Duration};

    use tempfile::TempDir;

    use super::*;
    use crate::config::{ResolvedConfig, WorkspaceConfig};

    fn empty_config() -> ResolvedConfig {
        ResolvedConfig {
            workspace: WorkspaceConfig::default(),
            tabs: Vec::new(),
        }
    }

    fn app(temp: &TempDir) -> App {
        App::new(temp.path().to_path_buf(), false, false, empty_config()).unwrap()
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
    fn markdown_raw_mode_switches_and_persists() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("README.md"), "# Hello").unwrap();
        fs::write(temp.path().join("notes.md"), "# Notes").unwrap();
        let mut app = app(&temp);

        assert!(app.markdown_rendered);
        app.toggle_markdown();
        assert!(!app.markdown_rendered);

        let notes = temp.path().canonicalize().unwrap().join("notes.md");
        app.select_path(&notes, true);
        assert!(!app.markdown_rendered);
    }

    #[test]
    fn markdown_preview_links_navigate_to_local_files() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("README.md"), "[Guide](docs/guide.md)").unwrap();
        fs::write(temp.path().join("docs/guide.md"), "# Guide").unwrap();
        let mut app = app(&temp);
        let root = temp.path().canonicalize().unwrap();
        let (tx, _rx) = crate::event::channel();

        app.select_path(&root.join("README.md"), true);
        app.preview.set_measurements(80, 10, 1);
        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE), &tx);
        let open = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);

        assert_eq!(open, None);
        assert_eq!(
            app.selected_path(),
            Some(root.join("docs/guide.md").as_path())
        );
    }

    #[test]
    fn markdown_preview_links_open_external_urls() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("README.md"), "[Web](https://example.com)").unwrap();
        let mut app = app(&temp);
        let root = temp.path().canonicalize().unwrap();
        let (tx, _rx) = crate::event::channel();

        app.select_path(&root.join("README.md"), true);
        app.preview.set_measurements(80, 10, 1);
        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE), &tx);
        let open = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);

        assert_eq!(
            open,
            Some(ExternalOpen::Url("https://example.com".to_string()))
        );
    }

    #[test]
    fn tree_refresh_preserves_selected_path_when_possible() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("docs/a.md"), "# A").unwrap();
        fs::write(temp.path().join("docs/b.md"), "# B").unwrap();

        let mut app = app(&temp);
        let root = temp.path().canonicalize().unwrap();
        let selected = root.join("docs/b.md");
        app.select_path(&selected, true);

        fs::write(temp.path().join("docs/aa.md"), "# AA").unwrap();
        app.reload_tree_preserving_selection();

        assert_eq!(app.selected_path(), Some(selected.as_path()));
    }

    #[test]
    fn tree_refresh_selects_existing_ancestor_after_deletion() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("docs/a.md"), "# A").unwrap();

        let mut app = app(&temp);
        let root = temp.path().canonicalize().unwrap();
        let selected = root.join("docs/a.md");
        app.select_path(&selected, true);

        fs::remove_file(temp.path().join("docs/a.md")).unwrap();
        app.reload_tree_preserving_selection();

        assert_eq!(app.selected_path(), Some(root.join("docs").as_path()));
    }

    #[test]
    fn validate_root_rejects_files() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("file.txt");
        fs::write(&file, "text").unwrap();

        assert!(validate_root(file).is_err());
    }

    #[test]
    fn configured_tabs_are_appended_after_files() {
        let temp = TempDir::new().unwrap();
        let config = ResolvedConfig {
            workspace: WorkspaceConfig::default(),
            tabs: vec![TerminalProfile {
                name: "Shell".to_string(),
                command: "sh".to_string(),
                args: Vec::new(),
                cwd: None,
                env: HashMap::new(),
                auto_start: false,
                restart_on_exit: false,
            }],
        };

        let app = App::new(temp.path().to_path_buf(), false, false, config).unwrap();

        assert_eq!(app.tabs[0].title, crate::tabs::RESERVED_FILES_TITLE);
        assert_eq!(app.tabs[1].title, "Shell");
    }

    #[test]
    fn q_requires_confirmation_before_quitting() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        let (tx, _rx) = crate::event::channel();

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE), &tx);

        assert_eq!(app.input_mode, InputMode::ConfirmQuit);
        assert!(!app.should_quit);

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE), &tx);

        assert_eq!(app.input_mode, InputMode::Repository);
        assert!(!app.should_quit);

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE), &tx);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);

        assert!(app.should_quit);
    }

    #[test]
    fn files_tab_direct_number_key_selects_configured_tab() {
        let temp = TempDir::new().unwrap();
        let mut app = App::new(
            temp.path().to_path_buf(),
            false,
            false,
            ResolvedConfig {
                workspace: WorkspaceConfig::default(),
                tabs: vec![TerminalProfile {
                    name: "Claude".to_string(),
                    command: "devdeck-missing-command-for-test".to_string(),
                    args: Vec::new(),
                    cwd: None,
                    env: HashMap::new(),
                    auto_start: false,
                    restart_on_exit: false,
                }],
            },
        )
        .unwrap();
        let (tx, _rx) = crate::event::channel();

        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE), &tx);

        assert_eq!(app.active_tab, 1);
        assert_eq!(app.input_mode, InputMode::Terminal);
        assert!(matches!(
            app.tabs[1].as_terminal().unwrap().state,
            TerminalTabState::Failed { .. }
        ));
    }

    #[test]
    fn unique_temporary_titles_get_numeric_suffixes() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        let profile = TerminalProfile {
            name: "parser.rs".to_string(),
            command: "sh".to_string(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            auto_start: false,
            restart_on_exit: false,
        };
        app.tabs
            .push(Tab::terminal_tab(TabId(99), profile.clone(), true));
        let mut second = profile;
        second.name = "parser.rs (2)".to_string();
        app.tabs.push(Tab::terminal_tab(TabId(100), second, true));

        assert_eq!(app.unique_temporary_title("parser.rs"), "parser.rs (3)");
    }

    #[cfg(unix)]
    #[test]
    fn v_binding_creates_temporary_editor_tab_for_selected_file() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("parser.rs");
        fs::write(&file, "fn main() {}\n").unwrap();
        let mut app = app(&temp);
        let root = temp.path().canonicalize().unwrap();
        app.select_path(&root.join("parser.rs"), true);

        let previous_visual = env::var_os("VISUAL");
        env::set_var("VISUAL", "/bin/cat");
        let (tx, _rx) = crate::event::channel();

        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE), &tx);

        if let Some(value) = previous_visual {
            env::set_var("VISUAL", value);
        } else {
            env::remove_var("VISUAL");
        }

        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active_tab, 1);
        assert!(app.tabs[1].temporary);
        assert_eq!(app.tabs[1].title, "parser.rs");
        assert_eq!(
            app.tabs[1].as_terminal().unwrap().profile.args,
            [root.join("parser.rs").to_string_lossy().to_string()]
        );
        assert!(app.tabs[1].return_to_files_on_exit);
        app.stop_all_sessions();
    }

    #[test]
    fn launcher_creates_temporary_command_tab() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        let (tx, _rx) = crate::event::channel();

        app.open_tab_launcher();
        app.launcher.name = "Echo".to_string();
        app.launcher.command = "printf hello".to_string();
        app.submit_launcher(&tx);

        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tabs[1].title, "Echo");
        assert!(app.tabs[1].temporary);
        let terminal = app.tabs[1].as_terminal().unwrap();
        assert_eq!(terminal.profile.command, "printf");
        assert_eq!(terminal.profile.args, ["hello"]);
        app.stop_all_sessions();
    }

    #[test]
    fn launcher_prefills_from_configured_profile_choice() {
        let temp = TempDir::new().unwrap();
        let mut env_values = HashMap::new();
        env_values.insert("CODEX_HOME".to_string(), "/tmp/codex".to_string());
        let mut app = App::new(
            temp.path().to_path_buf(),
            false,
            false,
            ResolvedConfig {
                workspace: WorkspaceConfig::default(),
                tabs: vec![TerminalProfile {
                    name: "Codex".to_string(),
                    command: "codex".to_string(),
                    args: vec!["--full-auto".to_string()],
                    cwd: None,
                    env: env_values.clone(),
                    auto_start: false,
                    restart_on_exit: false,
                }],
            },
        )
        .unwrap();

        app.open_tab_launcher();

        assert_eq!(app.launcher_choice_labels()[0], "Codex - codex --full-auto");
        assert_eq!(app.launcher.name, "Codex (2)");
        assert_eq!(app.launcher.command, "codex --full-auto");

        let (tx, _rx) = crate::event::channel();
        app.submit_launcher(&tx);

        assert_eq!(app.tabs.len(), 3);
        assert_eq!(app.tabs[2].title, "Codex (2)");
        assert!(app.tabs[2].temporary);
        let terminal = app.tabs[2].as_terminal().unwrap();
        assert_eq!(terminal.profile.command, "codex");
        assert_eq!(terminal.profile.args, ["--full-auto"]);
        assert_eq!(terminal.profile.env, env_values);
        app.stop_all_sessions();
    }

    #[test]
    fn launcher_can_create_new_shell_choice() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        let expected = parent_shell_profile(&app.selected_working_directory());
        let (tx, _rx) = crate::event::channel();

        app.open_tab_launcher();
        app.submit_launcher(&tx);

        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tabs[1].title, "Shell");
        assert!(app.tabs[1].temporary);
        let terminal = app.tabs[1].as_terminal().unwrap();
        assert_eq!(terminal.profile.command, expected.command);
        assert_eq!(terminal.profile.args, expected.args);
        app.stop_all_sessions();
    }

    #[test]
    fn launcher_prefills_cwd_from_selected_directory() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("docs/guide.md"), "# Guide").unwrap();
        let mut app = app(&temp);
        let root = temp.path().canonicalize().unwrap();

        app.select_path(&root.join("docs/guide.md"), true);
        app.open_tab_launcher();

        assert_eq!(app.launcher.cwd, "docs");
    }

    #[test]
    fn selected_working_directory_uses_selected_directory_or_file_parent() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("docs/guide.md"), "# Guide").unwrap();
        let mut app = app(&temp);
        let root = temp.path().canonicalize().unwrap();
        let docs = root.join("docs");

        app.select_path(&docs, true);
        assert_eq!(app.selected_working_directory(), docs);

        app.select_path(&root.join("docs/guide.md"), true);
        assert_eq!(app.selected_working_directory(), root.join("docs"));
    }

    #[test]
    fn codex_and_claude_profiles_without_cwd_launch_from_selected_directory() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("docs/guide.md"), "# Guide").unwrap();
        let mut app = app(&temp);
        let root = temp.path().canonicalize().unwrap();
        app.select_path(&root.join("docs/guide.md"), true);

        let mut codex = terminal_profile("Codex");
        codex.command = "codex".to_string();
        assert_eq!(app.command_spec_for_profile(&codex).cwd, root.join("docs"));

        let mut claude = terminal_profile("AI");
        claude.command = "/usr/local/bin/claude".to_string();
        assert_eq!(app.command_spec_for_profile(&claude).cwd, root.join("docs"));

        let shell = terminal_profile("Shell");
        assert_eq!(app.command_spec_for_profile(&shell).cwd, root);
    }

    #[test]
    fn explicit_agent_cwd_overrides_selected_directory() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("docs")).unwrap();
        fs::create_dir(temp.path().join("agent")).unwrap();
        fs::write(temp.path().join("docs/guide.md"), "# Guide").unwrap();
        let mut app = app(&temp);
        let root = temp.path().canonicalize().unwrap();
        app.select_path(&root.join("docs/guide.md"), true);

        let mut codex = terminal_profile("Codex");
        codex.command = "codex".to_string();
        codex.cwd = Some(root.join("agent"));

        assert_eq!(app.command_spec_for_profile(&codex).cwd, root.join("agent"));
    }

    #[test]
    fn inactive_terminal_output_marks_active_then_quiet() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        let session_id = SessionId(42);
        app.tabs.push(Tab::terminal_tab(
            TabId(2),
            terminal_profile("Codex"),
            false,
        ));
        let terminal = app.tabs[1].terminal_mut().unwrap();
        terminal.session_id = Some(session_id);
        terminal.state = TerminalTabState::Running;

        app.handle_pty_output(session_id, b"working\n");
        let ActivityState::OutputActive { last_output_at } = app.tabs[1].activity else {
            panic!("inactive output should mark the tab active");
        };

        app.update_background_activity(
            last_output_at + BACKGROUND_OUTPUT_QUIET_AFTER + Duration::from_millis(1),
        );

        assert_eq!(app.tabs[1].activity, ActivityState::OutputQuiet);
    }

    #[test]
    fn active_terminal_output_does_not_set_background_indicator() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        let session_id = SessionId(42);
        app.tabs.push(Tab::terminal_tab(
            TabId(2),
            terminal_profile("Codex"),
            false,
        ));
        app.active_tab = 1;
        let terminal = app.tabs[1].terminal_mut().unwrap();
        terminal.session_id = Some(session_id);
        terminal.state = TerminalTabState::Running;

        app.handle_pty_output(session_id, b"visible\n");

        assert_eq!(app.tabs[1].activity, ActivityState::None);
    }

    #[test]
    fn selecting_tab_clears_background_activity_indicator() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        app.tabs.push(Tab::terminal_tab(
            TabId(2),
            terminal_profile("Codex"),
            false,
        ));
        let terminal = app.tabs[1].terminal_mut().unwrap();
        terminal.state = TerminalTabState::Running;
        app.tabs[1].activity = ActivityState::OutputQuiet;
        let (tx, _rx) = crate::event::channel();

        app.select_tab(1, &tx);

        assert_eq!(app.tabs[1].activity, ActivityState::None);
    }

    #[test]
    fn rename_only_changes_temporary_tabs() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        let profile = TerminalProfile {
            name: "Temp".to_string(),
            command: "sh".to_string(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            auto_start: false,
            restart_on_exit: false,
        };
        app.tabs.push(Tab::terminal_tab(TabId(2), profile, true));
        app.active_tab = 1;

        app.open_rename_tab();
        app.rename.value = "Renamed".to_string();
        app.submit_rename();

        assert_eq!(app.tabs[1].title, "Renamed");
        assert_eq!(app.tabs[1].as_terminal().unwrap().profile.name, "Renamed");
    }

    #[test]
    fn exited_terminal_enter_retries_after_stale_session_id() {
        let temp = TempDir::new().unwrap();
        let mut app = App::new(
            temp.path().to_path_buf(),
            false,
            false,
            ResolvedConfig {
                workspace: WorkspaceConfig::default(),
                tabs: vec![TerminalProfile {
                    name: "Codex".to_string(),
                    command: "devdeck-missing-command-for-test".to_string(),
                    args: Vec::new(),
                    cwd: None,
                    env: HashMap::new(),
                    auto_start: false,
                    restart_on_exit: false,
                }],
            },
        )
        .unwrap();
        app.active_tab = 1;
        app.input_mode = InputMode::Terminal;
        let terminal = app.tabs[1].terminal_mut().unwrap();
        terminal.state = TerminalTabState::Exited { exit_code: Some(0) };
        terminal.session_id = Some(SessionId(42));
        let (tx, _rx) = crate::event::channel();

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);

        let terminal = app.tabs[1].as_terminal().unwrap();
        assert!(terminal.session_id.is_none());
        assert!(matches!(terminal.state, TerminalTabState::Failed { .. }));
    }

    #[test]
    fn spawn_failure_for_missing_executable_mentions_command() {
        let temp = TempDir::new().unwrap();
        let command = CommandSpec {
            executable: "devdeck-missing-command-for-test".to_string(),
            args: Vec::new(),
            cwd: temp.path().to_path_buf(),
            env: HashMap::new(),
        };
        let error = anyhow::anyhow!("Unable to start session 5");

        assert_eq!(
            spawn_failure_message(&command, &error),
            "Executable not found: devdeck-missing-command-for-test"
        );
    }

    #[test]
    fn config_reconcile_marks_running_changed_tabs_for_restart() {
        let temp = TempDir::new().unwrap();
        let mut app = App::new(
            temp.path().to_path_buf(),
            false,
            false,
            ResolvedConfig {
                workspace: WorkspaceConfig::default(),
                tabs: vec![TerminalProfile {
                    name: "Shell".to_string(),
                    command: "sh".to_string(),
                    args: Vec::new(),
                    cwd: None,
                    env: HashMap::new(),
                    auto_start: false,
                    restart_on_exit: false,
                }],
            },
        )
        .unwrap();
        if let Some(terminal) = app.tabs[1].terminal_mut() {
            terminal.state = TerminalTabState::Running;
        }

        let (_, changed, _) = app.reconcile_config(ResolvedConfig {
            workspace: WorkspaceConfig::default(),
            tabs: vec![TerminalProfile {
                name: "Shell".to_string(),
                command: "bash".to_string(),
                args: Vec::new(),
                cwd: None,
                env: HashMap::new(),
                auto_start: false,
                restart_on_exit: false,
            }],
        });

        assert_eq!(changed, 1);
        assert!(app.tabs[1].as_terminal().unwrap().requires_restart);
        assert_eq!(app.tabs[1].as_terminal().unwrap().profile.command, "bash");
    }

    #[test]
    fn exited_editor_tab_returns_focus_to_files_but_remains_visible() {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp);
        let profile = TerminalProfile {
            name: "README.md".to_string(),
            command: "cat".to_string(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            auto_start: false,
            restart_on_exit: false,
        };
        app.tabs.push(Tab::terminal_tab(TabId(2), profile, true));
        app.tabs[1].return_to_files_on_exit = true;
        app.active_tab = 1;
        let session_id = SessionId(42);
        app.tabs[1].terminal_mut().unwrap().session_id = Some(session_id);
        let (tx, _rx) = crate::event::channel();

        app.handle_terminal_exit(session_id, Some(0), &tx);

        assert_eq!(app.active_tab, 0);
        assert_eq!(app.tabs.len(), 2);
        assert!(matches!(
            app.tabs[1].as_terminal().unwrap().state,
            TerminalTabState::Exited { exit_code: Some(0) }
        ));
    }
}
