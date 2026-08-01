use std::time::Instant;

use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::{
    event::EventSender,
    pty::reader,
    session::{CommandSpec, ProcessStatus, SessionId, TerminalSession},
};

pub fn spawn_session(
    id: SessionId,
    title: String,
    command: CommandSpec,
    rows: u16,
    cols: u16,
    event_tx: EventSender,
) -> Result<TerminalSession> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut command_builder = CommandBuilder::new(&command.executable);
    command_builder.args(&command.args);
    command_builder.cwd(&command.cwd);
    for (key, value) in &command.env {
        command_builder.env(key, value);
    }

    let child = pair.slave.spawn_command(command_builder)?;
    drop(pair.slave);

    let reader = pair.master.try_clone_reader()?;
    let pty_writer = pair.master.take_writer()?;
    let pid = child.process_id();
    let reader_thread = reader::start_reader(id, reader, event_tx);

    Ok(TerminalSession {
        id,
        title,
        command,
        pty_master: pair.master,
        pty_writer,
        child,
        terminal: vt100::Parser::new(rows, cols, 0),
        rows,
        cols,
        process_status: ProcessStatus::Running,
        pid,
        last_activity: Some(Instant::now()),
        reader_thread: Some(reader_thread),
    })
}
