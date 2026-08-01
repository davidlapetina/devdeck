use std::{collections::HashMap, io::Write, time::Instant};

use anyhow::{Context, Result};

use crate::{
    event::EventSender,
    pty,
    session::{lifecycle, CommandSpec, ProcessStatus, SessionId, TerminalSession},
};

#[derive(Default)]
pub struct SessionRegistry {
    sessions: HashMap<SessionId, TerminalSession>,
    next_id: u64,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn start(
        &mut self,
        title: String,
        command: CommandSpec,
        rows: u16,
        cols: u16,
        event_tx: EventSender,
    ) -> Result<SessionId> {
        let id = SessionId(self.next_id);
        self.next_id += 1;
        let session = pty::spawn::spawn_session(id, title, command, rows, cols, event_tx)
            .with_context(|| format!("Unable to start session {}", id.0))?;
        self.sessions.insert(id, session);
        Ok(id)
    }

    pub fn write(&mut self, id: SessionId, bytes: &[u8]) -> Result<()> {
        let Some(session) = self.sessions.get_mut(&id) else {
            anyhow::bail!("session {} does not exist", id.0);
        };
        session.pty_writer.write_all(bytes)?;
        session.pty_writer.flush()?;
        Ok(())
    }

    pub fn handle_output(&mut self, id: SessionId, bytes: &[u8]) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.terminal.process(bytes);
            session.last_activity = Some(Instant::now());
        }
    }

    pub fn resize(&mut self, id: SessionId, rows: u16, cols: u16) -> Result<()> {
        let Some(session) = self.sessions.get_mut(&id) else {
            return Ok(());
        };
        if session.rows == rows && session.cols == cols {
            return Ok(());
        }
        pty::resize::resize_session(session, rows, cols)
    }

    pub fn session(&self, id: SessionId) -> Option<&TerminalSession> {
        self.sessions.get(&id)
    }

    pub fn session_mut(&mut self, id: SessionId) -> Option<&mut TerminalSession> {
        self.sessions.get_mut(&id)
    }

    pub fn poll_exits(&mut self) -> Vec<(SessionId, Option<i32>)> {
        let mut exited = Vec::new();
        for (id, session) in &mut self.sessions {
            if !matches!(session.process_status, ProcessStatus::Running) {
                continue;
            }
            match session.child.try_wait() {
                Ok(Some(status)) => {
                    let exit_code = Some(status.exit_code() as i32);
                    session.process_status = ProcessStatus::Exited { exit_code };
                    exited.push((*id, exit_code));
                }
                Ok(None) => {}
                Err(error) => {
                    session.process_status = ProcessStatus::Failed {
                        message: error.to_string(),
                    };
                }
            }
        }
        exited
    }

    pub fn stop(&mut self, id: SessionId) -> Result<()> {
        if let Some(session) = self.sessions.get_mut(&id) {
            lifecycle::stop_session(session)?;
        }
        Ok(())
    }

    pub fn remove(&mut self, id: SessionId) -> Option<TerminalSession> {
        self.sessions.remove(&id)
    }

    pub fn stop_all(&mut self) {
        let ids = self.sessions.keys().copied().collect::<Vec<_>>();
        for id in ids {
            let _ = self.stop(id);
            self.sessions.remove(&id);
        }
    }
}

impl Drop for SessionRegistry {
    fn drop(&mut self) {
        self.stop_all();
    }
}
