use std::{collections::HashMap, io::Write, time::Instant};

use anyhow::{Context, Result};

use crate::{
    event::EventSender,
    pty,
    session::{
        lifecycle, CommandSpec, ProcessStatus, SessionId, TerminalPromptState, TerminalSession,
    },
};

const BRACKETED_PASTE_ENABLE: &[u8] = b"\x1b[?2004h";
const BRACKETED_PASTE_DISABLE: &[u8] = b"\x1b[?2004l";
const OSC_133_PREFIX: &[u8] = b"\x1b]133;";
const OUTPUT_TAIL_LIMIT: usize = 32;

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
            update_terminal_output_metadata(session, bytes);
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

fn update_terminal_output_metadata(session: &mut TerminalSession, bytes: &[u8]) {
    let mut combined = Vec::with_capacity(session.output_tail.len() + bytes.len());
    combined.extend_from_slice(&session.output_tail);
    combined.extend_from_slice(bytes);

    update_bracketed_paste_mode(session, &combined);
    update_prompt_state(session, &combined);

    let tail_start = combined.len().saturating_sub(OUTPUT_TAIL_LIMIT);
    session.output_tail = combined[tail_start..].to_vec();
}

fn update_bracketed_paste_mode(session: &mut TerminalSession, combined: &[u8]) {
    let enable_at = find_last(&combined, BRACKETED_PASTE_ENABLE);
    let disable_at = find_last(&combined, BRACKETED_PASTE_DISABLE);
    match (enable_at, disable_at) {
        (Some(enable), Some(disable)) => session.bracketed_paste_enabled = enable > disable,
        (Some(_), None) => session.bracketed_paste_enabled = true,
        (None, Some(_)) => session.bracketed_paste_enabled = false,
        (None, None) => {}
    }
}

fn update_prompt_state(session: &mut TerminalSession, combined: &[u8]) {
    let Some(marker) = last_osc_133_marker(combined) else {
        return;
    };
    session.prompt_state = match marker {
        b'A' | b'B' => TerminalPromptState::AtPrompt,
        b'C' => TerminalPromptState::CommandRunning,
        b'D' => TerminalPromptState::Unknown,
        _ => session.prompt_state,
    };
}

fn last_osc_133_marker(bytes: &[u8]) -> Option<u8> {
    let mut index = 0;
    let mut marker = None;
    while index + OSC_133_PREFIX.len() < bytes.len() {
        let Some(relative_start) = bytes[index..]
            .windows(OSC_133_PREFIX.len())
            .position(|window| window == OSC_133_PREFIX)
        else {
            break;
        };
        let start = index + relative_start;
        let payload_start = start + OSC_133_PREFIX.len();
        let payload = &bytes[payload_start..];
        if let Some(end) = osc_payload_end(payload) {
            if !payload.is_empty() {
                marker = Some(payload[0]);
            }
            index = payload_start + end;
        } else {
            break;
        }
    }
    marker
}

fn osc_payload_end(payload: &[u8]) -> Option<usize> {
    let bel = payload.iter().position(|byte| *byte == 0x07);
    let st = payload.windows(2).position(|window| window == b"\x1b\\");
    match (bel, st) {
        (Some(bel), Some(st)) => Some(bel.min(st)),
        (Some(bel), None) => Some(bel),
        (None, Some(st)) => Some(st),
        (None, None) => None,
    }
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

impl Drop for SessionRegistry {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_osc_133_prompt_markers_with_bel_or_st() {
        assert_eq!(last_osc_133_marker(b"\x1b]133;A\x07"), Some(b'A'));
        assert_eq!(last_osc_133_marker(b"\x1b]133;B\x1b\\"), Some(b'B'));
        assert_eq!(last_osc_133_marker(b"\x1b]133;D;127\x07"), Some(b'D'));
    }

    #[test]
    fn parses_the_last_complete_osc_133_marker() {
        assert_eq!(
            last_osc_133_marker(b"\x1b]133;B\x07prompt\x1b]133;C\x07"),
            Some(b'C')
        );
    }

    #[test]
    fn ignores_incomplete_osc_133_markers() {
        assert_eq!(last_osc_133_marker(b"\x1b]133;B"), None);
    }
}
