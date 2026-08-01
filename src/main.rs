use std::{
    env,
    io::{self, Stdout},
    path::Path,
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self as crossterm_event, Event as CrosstermEvent},
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

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
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
    let dimensions = current_terminal_dimensions(&terminal)?;
    app.initialize_terminal_tabs(&event_tx, dimensions);
    let result = run_loop(&mut terminal, &mut app, event_tx, event_rx);
    app.stop_all_sessions();
    restore_terminal(&mut terminal)?;
    result
}

fn run_loop(
    terminal: &mut AppTerminal,
    app: &mut App,
    event_tx: event::EventSender,
    event_rx: event::EventReceiver,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        if app.should_quit {
            break;
        }

        if crossterm_event::poll(event::TICK_RATE)? {
            match crossterm_event::read()? {
                CrosstermEvent::Key(key) => {
                    handle_app_event(terminal, app, &event_tx, AppEvent::Key(key))?
                }
                CrosstermEvent::Resize(width, height) => {
                    handle_app_event(terminal, app, &event_tx, AppEvent::Resize { width, height })?
                }
                CrosstermEvent::Mouse(mouse) => {
                    handle_app_event(terminal, app, &event_tx, AppEvent::Mouse(mouse))?
                }
                CrosstermEvent::FocusGained
                | CrosstermEvent::FocusLost
                | CrosstermEvent::Paste(_) => {}
            }
        } else {
            handle_app_event(terminal, app, &event_tx, AppEvent::Tick)?;
        }

        while let Ok(event) = event_rx.try_recv() {
            handle_app_event(terminal, app, &event_tx, event)?;
        }
    }

    Ok(())
}

fn handle_app_event(
    terminal: &mut AppTerminal,
    app: &mut App,
    event_tx: &event::EventSender,
    event: AppEvent,
) -> Result<()> {
    match event {
        AppEvent::Key(key) => {
            if let Some(open) = app.handle_key(key, event_tx) {
                handle_external_open(terminal, app, open)?;
            }
        }
        AppEvent::Resize { width, height } => {
            let area = Rect::new(0, 0, width, height);
            let dimensions = ui::layout::terminal_dimensions(ui::layout::areas(area).content);
            app.resize_active_terminal(dimensions);
        }
        AppEvent::Mouse(_) => {}
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

fn current_terminal_dimensions(terminal: &AppTerminal) -> Result<devdeck::app::TerminalDimensions> {
    let size = terminal.size()?;
    let area = Rect::new(0, 0, size.width, size.height);
    Ok(ui::layout::terminal_dimensions(
        ui::layout::areas(area).content,
    ))
}

fn setup_terminal() -> Result<AppTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut AppTerminal) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn resume_terminal(terminal: &mut AppTerminal) -> Result<()> {
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.hide_cursor()?;
    terminal.clear()?;
    Ok(())
}

fn handle_external_open(
    terminal: &mut AppTerminal,
    app: &mut App,
    open: ExternalOpen,
) -> Result<()> {
    let Some(path) = app.selected_external_path() else {
        return Ok(());
    };

    if matches!(open, ExternalOpen::Editor) && !path.is_file() {
        app.set_status("Selected entry is not a file");
        return Ok(());
    }

    restore_terminal(terminal)?;
    let result = match open {
        ExternalOpen::Editor => open_in_editor(&path),
        ExternalOpen::OperatingSystem => open_with_os(&path),
    };
    std::thread::sleep(Duration::from_millis(25));
    resume_terminal(terminal)?;

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
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(path).status()?
    } else if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .status()?
    } else {
        Command::new("xdg-open").arg(path).status()?
    };

    if !status.success() {
        anyhow::bail!("open command exited with {status}");
    }

    Ok(())
}
