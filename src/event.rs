use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
    time::Duration,
};

use crossterm::event::{KeyEvent, MouseEvent};

use crate::{config::ResolvedConfig, session::SessionId};

#[derive(Debug, Clone)]
pub struct FsEventBatch {
    pub paths: Vec<PathBuf>,
    pub tree_changed: bool,
    /// Source/destination rename pairs reported within this debounced batch.
    pub renames: Vec<(PathBuf, PathBuf)>,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Paste(String),
    Mouse(MouseEvent),
    Resize {
        width: u16,
        height: u16,
    },
    FileSystem(FsEventBatch),
    PtyOutput {
        session_id: SessionId,
        bytes: Vec<u8>,
    },
    ProcessExited {
        session_id: SessionId,
        exit_code: Option<i32>,
    },
    ProcessFailed {
        session_id: SessionId,
        message: String,
    },
    ConfigReloaded {
        config: ResolvedConfig,
    },
    ConfigReloadFailed {
        message: String,
    },
    Tick,
}

pub type EventSender = SyncSender<AppEvent>;
pub type EventReceiver = Receiver<AppEvent>;

pub fn channel() -> (EventSender, EventReceiver) {
    mpsc::sync_channel(EVENT_CHANNEL_CAPACITY)
}

pub const EVENT_CHANNEL_CAPACITY: usize = 1_024;
pub const TICK_RATE: Duration = Duration::from_millis(250);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_event_channel_has_a_hard_capacity() {
        let (tx, _rx) = channel();
        for _ in 0..EVENT_CHANNEL_CAPACITY {
            tx.try_send(AppEvent::Tick).unwrap();
        }
        assert!(matches!(
            tx.try_send(AppEvent::Tick),
            Err(mpsc::TrySendError::Full(_))
        ));
    }
}
