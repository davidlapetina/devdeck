use std::{thread, time::Duration};

use anyhow::Result;

use crate::session::TerminalSession;

pub const STOP_WAIT: Duration = Duration::from_millis(500);

pub fn stop_session(session: &mut TerminalSession) -> Result<()> {
    if matches!(
        session.process_status,
        crate::session::ProcessStatus::Running
    ) {
        let _ = session.child.kill();
        thread::sleep(STOP_WAIT);
        let _ = session.child.try_wait();
    }
    Ok(())
}
