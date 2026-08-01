use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

use crossterm::event::{KeyEvent, MouseEvent};

use crate::{config::ResolvedConfig, session::SessionId};

#[derive(Debug, Clone)]
pub struct FsEventBatch {
    pub paths: Vec<PathBuf>,
    pub tree_changed: bool,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
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

pub type EventSender = Sender<AppEvent>;
pub type EventReceiver = Receiver<AppEvent>;

pub fn channel() -> (EventSender, EventReceiver) {
    mpsc::channel()
}

pub const TICK_RATE: Duration = Duration::from_millis(250);
