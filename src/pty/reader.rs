use std::{io::Read, thread};

use crate::{
    event::{AppEvent, EventSender},
    session::SessionId,
};

pub fn start_reader(
    session_id: SessionId,
    mut reader: Box<dyn Read + Send>,
    event_tx: EventSender,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let _ = event_tx.send(AppEvent::PtyOutput {
                        session_id,
                        bytes: buffer[..read].to_vec(),
                    });
                }
                Err(_) => break,
            }
        }
    })
}
