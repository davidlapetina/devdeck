use anyhow::Result;
use portable_pty::PtySize;

use crate::session::TerminalSession;

pub fn resize_session(session: &mut TerminalSession, rows: u16, cols: u16) -> Result<()> {
    session.pty_master.resize(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    session.terminal.set_size(rows, cols);
    session.rows = rows;
    session.cols = cols;
    Ok(())
}
