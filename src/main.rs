use std::{
    env,
    ffi::OsStr,
    io::{self, Stdout},
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    cursor::Show,
    event::{
        self as crossterm_event, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
        EnableMouseCapture, Event as CrosstermEvent, KeyboardEnhancementFlags, MouseButton,
        MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use devdeck::{
    app::{validate_root, App, ExternalOpen},
    cli::Cli,
    config,
    event::{self, AppEvent},
    filesystem::watcher,
    ui,
};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};

type AppTerminal = Terminal<CrosstermBackend<Stdout>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseTarget {
    Tabs,
    Tree { visible_row: usize },
    Preview,
    Terminal,
    Border,
    Outside,
}

static TERMINAL_MODES_ACTIVE: AtomicBool = AtomicBool::new(false);

fn main() {
    install_terminal_panic_hook();
    if let Err(error) = ensure_macos_malloc_nano_zone_disabled().and_then(|()| run()) {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "macos")]
fn ensure_macos_malloc_nano_zone_disabled() -> Result<()> {
    if env::var_os("MallocNanoZone").is_some() {
        return Ok(());
    }

    use std::os::unix::process::CommandExt;

    let executable = env::current_exe().context("Unable to resolve current executable")?;
    let error = Command::new(executable)
        .args(env::args_os().skip(1))
        .env("MallocNanoZone", "0")
        .exec();
    Err(error).context("Unable to restart DevDeck with MallocNanoZone=0")
}

#[cfg(not(target_os = "macos"))]
fn ensure_macos_malloc_nano_zone_disabled() -> Result<()> {
    Ok(())
}

fn install_terminal_panic_hook() {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        restore_terminal_modes_from_stdout();
        previous_hook(panic_info);
    }));
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = validate_root(cli.path)?;
    let resolved_config = config::load_config(&root)?;
    let mut app = App::new(root.clone(), cli.hidden, !cli.no_watch, resolved_config)?;
    let (event_tx, event_rx) = event::channel();
    let _watcher = if app.watch_enabled {
        match watcher::start(root, event_tx.clone(), watcher::DEFAULT_DEBOUNCE) {
            Ok(handle) => Some(handle),
            Err(error) => {
                app.watch_enabled = false;
                app.set_status(format!("Watcher unavailable: {error}"));
                None
            }
        }
    } else {
        None
    };

    let mut terminal = setup_terminal().context("Unable to initialize terminal")?;
    let result = (|| {
        let dimensions = current_terminal_dimensions(&terminal)?;
        app.initialize_terminal_tabs(&event_tx, dimensions);
        run_loop(&mut terminal, &mut app, event_tx, event_rx)
    })();
    app.stop_all_sessions();
    let restore_result = restore_terminal(&mut terminal);
    match (result, restore_result) {
        (Err(error), Err(restore_error)) => {
            Err(error.context(format!("Terminal cleanup also failed: {restore_error:#}")))
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error).context("Unable to restore terminal"),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn run_loop(
    terminal: &mut AppTerminal,
    app: &mut App,
    event_tx: event::EventSender,
    event_rx: event::EventReceiver,
) -> Result<()> {
    let desired_mouse_capture = app.effective_mouse_capture();
    // Start deliberately out of sync so startup always emits the desired mode. In particular,
    // Selection and Auto-on-terminal must disable capture left behind by a previous program.
    let mut terminal_mouse_capture = !desired_mouse_capture;
    sync_mouse_capture(terminal, &mut terminal_mouse_capture, desired_mouse_capture)?;

    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        if app.should_quit {
            break;
        }

        if crossterm_event::poll(event::TICK_RATE)? {
            match crossterm_event::read()? {
                CrosstermEvent::Key(key) => handle_app_event(
                    terminal,
                    app,
                    &event_tx,
                    &mut terminal_mouse_capture,
                    AppEvent::Key(key),
                )?,
                CrosstermEvent::Resize(width, height) => handle_app_event(
                    terminal,
                    app,
                    &event_tx,
                    &mut terminal_mouse_capture,
                    AppEvent::Resize { width, height },
                )?,
                CrosstermEvent::Mouse(mouse) => handle_app_event(
                    terminal,
                    app,
                    &event_tx,
                    &mut terminal_mouse_capture,
                    AppEvent::Mouse(mouse),
                )?,
                CrosstermEvent::Paste(text) => handle_app_event(
                    terminal,
                    app,
                    &event_tx,
                    &mut terminal_mouse_capture,
                    AppEvent::Paste(text),
                )?,
                CrosstermEvent::FocusGained | CrosstermEvent::FocusLost => {}
            }
        } else {
            handle_app_event(
                terminal,
                app,
                &event_tx,
                &mut terminal_mouse_capture,
                AppEvent::Tick,
            )?;
        }

        while let Ok(event) = event_rx.try_recv() {
            handle_app_event(terminal, app, &event_tx, &mut terminal_mouse_capture, event)?;
        }

        sync_mouse_capture(
            terminal,
            &mut terminal_mouse_capture,
            app.effective_mouse_capture(),
        )?;
    }

    Ok(())
}

fn handle_app_event(
    terminal: &mut AppTerminal,
    app: &mut App,
    event_tx: &event::EventSender,
    terminal_mouse_capture: &mut bool,
    event: AppEvent,
) -> Result<()> {
    match event {
        AppEvent::Key(key) => {
            if let Some(open) = app.handle_key(key, event_tx) {
                handle_external_open(terminal, app, terminal_mouse_capture, open)?;
            }
        }
        AppEvent::Paste(text) => app.handle_paste(text),
        AppEvent::Resize { width, height } => {
            let area = Rect::new(0, 0, width, height);
            let dimensions = ui::layout::terminal_dimensions(ui::layout::areas(area).content);
            app.resize_active_terminal(dimensions);
        }
        AppEvent::Mouse(mouse) => {
            if !app.effective_mouse_capture() {
                app.clear_mouse_click_tracking();
                return Ok(());
            }
            if mouse_event_breaks_click_chain(mouse.kind) {
                app.clear_mouse_click_tracking();
            }
            let size = terminal.size()?;
            let area = Rect::new(0, 0, size.width, size.height);
            let areas = ui::layout::areas_for_files(area, app.maximized_files_pane);
            let target = mouse_target(areas, mouse.column, mouse.row, app.input_mode);
            match target {
                MouseTarget::Tabs if app.accepts_mouse_navigation() => {
                    let index = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                        .then(|| ui::tabs::hit_test(areas.tabs, app, mouse.column, mouse.row))
                        .flatten();
                    app.handle_tab_mouse(index, mouse, event_tx);
                }
                MouseTarget::Tree { visible_row } => {
                    let inner_height = areas.files.height.saturating_sub(2) as usize;
                    app.handle_tree_mouse(mouse, Some(visible_row), inner_height);
                }
                MouseTarget::Preview => {
                    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                        app.clear_mouse_click_tracking();
                    }
                    app.handle_preview_mouse(mouse);
                }
                MouseTarget::Terminal => {
                    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                        app.clear_mouse_click_tracking();
                    }
                    app.handle_terminal_mouse(mouse);
                }
                MouseTarget::Tabs | MouseTarget::Border | MouseTarget::Outside
                    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) =>
                {
                    app.clear_mouse_click_tracking();
                }
                _ => {}
            }
        }
        AppEvent::FileSystem(batch) => app.handle_filesystem_event(batch),
        AppEvent::PtyOutput { session_id, bytes } => app.handle_pty_output(session_id, &bytes),
        AppEvent::ProcessExited {
            session_id: _,
            exit_code: _,
        } => {}
        AppEvent::ProcessFailed {
            session_id: _,
            message,
        } => app.set_status(format!("Process failed: {message}")),
        AppEvent::ConfigReloaded { config: _ } => {}
        AppEvent::ConfigReloadFailed { message } => {
            app.set_status(format!("Configuration reload failed: {message}"))
        }
        AppEvent::Tick => app.tick(event_tx),
    }
    Ok(())
}

fn mouse_event_breaks_click_chain(kind: MouseEventKind) -> bool {
    match kind {
        MouseEventKind::ScrollUp
        | MouseEventKind::ScrollDown
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight => true,
        MouseEventKind::Down(button) => button != MouseButton::Left,
        MouseEventKind::Up(_) | MouseEventKind::Drag(_) | MouseEventKind::Moved => false,
    }
}

fn current_terminal_dimensions(terminal: &AppTerminal) -> Result<devdeck::app::TerminalDimensions> {
    let size = terminal.size()?;
    let area = Rect::new(0, 0, size.width, size.height);
    Ok(ui::layout::terminal_dimensions(
        ui::layout::areas(area).content,
    ))
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

fn point_in_inner(rect: Rect, column: u16, row: u16) -> bool {
    rect.width > 2
        && rect.height > 2
        && column > rect.x
        && column < rect.x.saturating_add(rect.width).saturating_sub(1)
        && row > rect.y
        && row < rect.y.saturating_add(rect.height).saturating_sub(1)
}

fn mouse_target(
    areas: ui::layout::AppAreas,
    column: u16,
    row: u16,
    input_mode: devdeck::app::InputMode,
) -> MouseTarget {
    if rect_contains(areas.tabs, column, row) {
        return MouseTarget::Tabs;
    }
    match input_mode {
        devdeck::app::InputMode::Repository if rect_contains(areas.files, column, row) => {
            if point_in_inner(areas.files, column, row) {
                MouseTarget::Tree {
                    visible_row: row.saturating_sub(areas.files.y + 1) as usize,
                }
            } else {
                MouseTarget::Border
            }
        }
        devdeck::app::InputMode::Repository if rect_contains(areas.preview, column, row) => {
            if point_in_inner(areas.preview, column, row) {
                MouseTarget::Preview
            } else {
                MouseTarget::Border
            }
        }
        devdeck::app::InputMode::Terminal if rect_contains(areas.content, column, row) => {
            if point_in_inner(areas.content, column, row) {
                MouseTarget::Terminal
            } else {
                MouseTarget::Border
            }
        }
        _ => MouseTarget::Outside,
    }
}

fn setup_terminal() -> Result<AppTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    let setup_result = (|| {
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
        // Mark before writing the push sequence so cleanup also pops after a partial write/error.
        TERMINAL_MODES_ACTIVE.store(true, Ordering::SeqCst);
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.hide_cursor()?;
        Ok(terminal)
    })();

    if setup_result.is_err() {
        restore_terminal_modes_from_stdout();
    }

    setup_result
}

fn restore_terminal(terminal: &mut AppTerminal) -> Result<()> {
    let modes_result = restore_terminal_modes(terminal.backend_mut());
    let cursor_result = terminal.show_cursor();
    modes_result?;
    cursor_result?;
    Ok(())
}

fn resume_terminal(terminal: &mut AppTerminal, mouse_capture_enabled: bool) -> Result<()> {
    enable_raw_mode()?;
    let result = (|| {
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableBracketedPaste
        )?;
        TERMINAL_MODES_ACTIVE.store(true, Ordering::SeqCst);
        execute!(
            terminal.backend_mut(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        set_mouse_capture(terminal.backend_mut(), mouse_capture_enabled)?;
        terminal.hide_cursor()?;
        terminal.clear()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = restore_terminal(terminal);
    }
    result
}

fn sync_mouse_capture(
    terminal: &mut AppTerminal,
    terminal_mouse_capture: &mut bool,
    app_mouse_capture: bool,
) -> Result<()> {
    sync_mouse_capture_writer(
        terminal.backend_mut(),
        terminal_mouse_capture,
        app_mouse_capture,
    )
}

fn sync_mouse_capture_writer<W: io::Write>(
    writer: &mut W,
    terminal_mouse_capture: &mut bool,
    app_mouse_capture: bool,
) -> Result<()> {
    if *terminal_mouse_capture == app_mouse_capture {
        return Ok(());
    }

    set_mouse_capture(writer, app_mouse_capture)?;
    *terminal_mouse_capture = app_mouse_capture;
    Ok(())
}

fn set_mouse_capture<W: io::Write>(writer: &mut W, enabled: bool) -> Result<()> {
    if enabled {
        execute!(writer, EnableMouseCapture)?;
    } else {
        execute!(writer, DisableMouseCapture)?;
    }
    Ok(())
}

fn restore_terminal_modes<W: io::Write>(writer: &mut W) -> Result<()> {
    let raw_mode_result = disable_raw_mode();
    let pop_keyboard_enhancement = TERMINAL_MODES_ACTIVE.swap(false, Ordering::SeqCst);
    let terminal_result = write_terminal_restore_sequences(writer, pop_keyboard_enhancement);

    raw_mode_result?;
    terminal_result?;
    Ok(())
}

fn write_terminal_restore_sequences<W: io::Write>(
    writer: &mut W,
    pop_keyboard_enhancement: bool,
) -> Result<()> {
    if pop_keyboard_enhancement {
        execute!(
            writer,
            DisableBracketedPaste,
            DisableMouseCapture,
            PopKeyboardEnhancementFlags,
            LeaveAlternateScreen
        )?;
    } else {
        execute!(
            writer,
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        )?;
    }
    Ok(())
}

fn restore_terminal_modes_from_stdout() {
    let mut stdout = io::stdout();
    let _ = restore_terminal_modes(&mut stdout);
    let _ = execute!(stdout, Show);
}

fn handle_external_open(
    terminal: &mut AppTerminal,
    app: &mut App,
    terminal_mouse_capture: &mut bool,
    open: ExternalOpen,
) -> Result<()> {
    let editor_open = match open {
        ExternalOpen::Url(target) => {
            restore_terminal(terminal)?;
            *terminal_mouse_capture = false;
            let result = open_with_os_arg(&target);
            std::thread::sleep(Duration::from_millis(25));
            resume_terminal(terminal, app.effective_mouse_capture())?;
            *terminal_mouse_capture = app.effective_mouse_capture();

            match result {
                Ok(()) => app.set_status(format!("Opened: {target}")),
                Err(error) => app.set_status(format!("Open failed: {error:#}")),
            }

            return Ok(());
        }
        ExternalOpen::Editor => true,
        ExternalOpen::OperatingSystem => false,
    };

    let Some(path) = app.selected_external_path() else {
        return Ok(());
    };

    if editor_open && !path.is_file() {
        app.set_status("Selected entry is not a file");
        return Ok(());
    }

    restore_terminal(terminal)?;
    *terminal_mouse_capture = false;
    let result = if editor_open {
        open_in_editor(&path)
    } else {
        open_with_os(&path)
    };
    std::thread::sleep(Duration::from_millis(25));
    resume_terminal(terminal, app.effective_mouse_capture())?;
    *terminal_mouse_capture = app.effective_mouse_capture();

    match result {
        Ok(()) => app.set_status(format!("Opened: {}", app.relative_display(&path))),
        Err(error) => app.set_status(format!("Open failed: {error:#}")),
    }

    Ok(())
}

fn open_in_editor(path: &Path) -> Result<()> {
    let editor = env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_string());
    let mut parts = shell_words::split(&editor)
        .with_context(|| format!("Invalid editor command: {editor:?}"))?;
    let command = parts.first().cloned().context("EDITOR is empty")?;
    parts.remove(0);
    let status = Command::new(command).args(parts).arg(path).status()?;
    if !status.success() {
        anyhow::bail!("editor exited with {status}");
    }
    Ok(())
}

fn open_with_os(path: &Path) -> Result<()> {
    open_with_os_arg(path.as_os_str())
}

fn open_with_os_arg(target: impl AsRef<OsStr>) -> Result<()> {
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(target.as_ref()).status()?
    } else if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(target.as_ref())
            .status()?
    } else {
        Command::new("xdg-open").arg(target.as_ref()).status()?
    };

    if !status.success() {
        anyhow::bail!("open command exited with {status}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use devdeck::app::InputMode;

    #[test]
    fn coordinate_routing_distinguishes_panes_and_borders() {
        let areas = ui::layout::areas(Rect::new(0, 0, 100, 30));

        assert_eq!(
            mouse_target(areas, 4, 1, InputMode::Repository),
            MouseTarget::Tabs
        );
        assert_eq!(
            mouse_target(
                areas,
                areas.files.x + 1,
                areas.files.y + 2,
                InputMode::Repository
            ),
            MouseTarget::Tree { visible_row: 1 }
        );
        assert_eq!(
            mouse_target(
                areas,
                areas.files.x,
                areas.files.y + 2,
                InputMode::Repository
            ),
            MouseTarget::Border
        );
        assert_eq!(
            mouse_target(
                areas,
                areas.preview.x + 1,
                areas.preview.y + 1,
                InputMode::Repository
            ),
            MouseTarget::Preview
        );
        assert_eq!(
            mouse_target(
                areas,
                areas.content.x + 1,
                areas.content.y + 1,
                InputMode::Terminal
            ),
            MouseTarget::Terminal
        );
        assert_eq!(
            mouse_target(
                areas,
                areas.content.x,
                areas.content.y + 1,
                InputMode::Terminal
            ),
            MouseTarget::Border
        );
        assert_eq!(
            mouse_target(areas, 99, 29, InputMode::Repository),
            MouseTarget::Outside
        );
    }

    #[test]
    fn coordinate_routing_matches_maximized_files_layout() {
        use devdeck::app::FilesPane;

        let area = Rect::new(0, 0, 100, 30);
        let preview = ui::layout::areas_for_files(area, Some(FilesPane::Preview));
        assert_eq!(
            mouse_target(
                preview,
                preview.content.x + 1,
                preview.content.y + 1,
                InputMode::Repository
            ),
            MouseTarget::Preview
        );

        let tree = ui::layout::areas_for_files(area, Some(FilesPane::Tree));
        assert_eq!(
            mouse_target(
                tree,
                tree.content.x + 1,
                tree.content.y + 1,
                InputMode::Repository
            ),
            MouseTarget::Tree { visible_row: 0 }
        );
    }

    #[test]
    fn disabled_mouse_capture_emits_an_explicit_disable_sequence() {
        let mut output = Vec::new();

        set_mouse_capture(&mut output, false).unwrap();

        assert_eq!(
            output,
            b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l"
        );
    }

    #[test]
    fn disabled_startup_state_is_explicitly_synchronized() {
        let mut output = Vec::new();
        let desired = false;
        let mut tracked = !desired;

        sync_mouse_capture_writer(&mut output, &mut tracked, desired).unwrap();

        assert!(!tracked);
        assert_eq!(
            output,
            b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l"
        );
    }

    #[test]
    fn mouse_release_motion_and_drag_preserve_click_chain_but_scroll_breaks_it() {
        assert!(!mouse_event_breaks_click_chain(MouseEventKind::Up(
            MouseButton::Left
        )));
        assert!(!mouse_event_breaks_click_chain(MouseEventKind::Moved));
        assert!(!mouse_event_breaks_click_chain(MouseEventKind::Drag(
            MouseButton::Left
        )));
        assert!(mouse_event_breaks_click_chain(MouseEventKind::ScrollDown));
        assert!(mouse_event_breaks_click_chain(MouseEventKind::Down(
            MouseButton::Right
        )));
    }

    #[test]
    fn active_terminal_cleanup_pops_keyboard_enhancement_flags() {
        let mut output = Vec::new();
        write_terminal_restore_sequences(&mut output, true).unwrap();
        assert!(output
            .windows(b"\x1b[<1u".len())
            .any(|window| window == b"\x1b[<1u"));

        output.clear();
        write_terminal_restore_sequences(&mut output, false).unwrap();
        assert!(!output
            .windows(b"\x1b[<1u".len())
            .any(|window| window == b"\x1b[<1u"));
    }
}
