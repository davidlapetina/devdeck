use std::{collections::HashMap, io::Write, path::PathBuf, thread::JoinHandle, time::Instant};

use portable_pty::{Child, MasterPty};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub executable: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessStatus {
    Running,
    Exited { exit_code: Option<i32> },
    Failed { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalPromptState {
    Unknown,
    AtPrompt,
    CommandRunning,
}

pub struct TerminalSession {
    pub id: SessionId,
    pub title: String,
    pub command: CommandSpec,
    pub pty_master: Box<dyn MasterPty + Send>,
    pub pty_writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send>,
    pub terminal: vt100::Parser,
    pub rows: u16,
    pub cols: u16,
    pub process_status: ProcessStatus,
    pub pid: Option<u32>,
    pub last_activity: Option<Instant>,
    pub reader_thread: Option<JoinHandle<()>>,
    pub bracketed_paste_enabled: bool,
    pub prompt_state: TerminalPromptState,
    pub output_tail: Vec<u8>,
}
