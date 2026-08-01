#![cfg(unix)]

use std::{
    collections::HashMap,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use devdeck::{
    event::{self, AppEvent},
    session::{CommandSpec, SessionId, SessionRegistry},
};
use tempfile::TempDir;

#[test]
fn pty_session_runs_shell_command_receives_output_resizes_and_exits() {
    let temp = TempDir::new().unwrap();
    let (tx, rx) = event::channel();
    let mut registry = SessionRegistry::new();
    let command = CommandSpec {
        executable: "/bin/sh".to_string(),
        args: Vec::new(),
        cwd: temp.path().to_path_buf(),
        env: HashMap::from([
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
            ("DEVDECK".to_string(), "1".to_string()),
        ]),
    };

    let session_id = registry
        .start("Shell".to_string(), command, 20, 80, tx)
        .unwrap();
    registry.resize(session_id, 12, 40).unwrap();
    assert_eq!(registry.session(session_id).unwrap().rows, 12);
    assert_eq!(registry.session(session_id).unwrap().cols, 40);

    registry
        .write(session_id, b"printf DEVDECK_PTY_OK\\n\r")
        .unwrap();
    let output = wait_for_output(&rx, &mut registry, session_id, "DEVDECK_PTY_OK");
    assert!(output.contains("DEVDECK_PTY_OK"));

    registry.write(session_id, b"exit\r").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut exited = false;
    while Instant::now() < deadline {
        drain_events(&rx, &mut registry);
        if registry
            .poll_exits()
            .iter()
            .any(|(id, _)| *id == session_id)
        {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    registry.stop_all();
    assert!(exited, "shell did not exit within timeout");
}

fn wait_for_output(
    rx: &Receiver<AppEvent>,
    registry: &mut SessionRegistry,
    session_id: SessionId,
    expected: &str,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut output = String::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(AppEvent::PtyOutput {
                session_id: id,
                bytes,
            }) => {
                registry.handle_output(id, &bytes);
                if id == session_id {
                    output.push_str(&String::from_utf8_lossy(&bytes));
                    if output.contains(expected) {
                        return output;
                    }
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }
    output
}

fn drain_events(rx: &Receiver<AppEvent>, registry: &mut SessionRegistry) {
    while let Ok(event) = rx.try_recv() {
        if let AppEvent::PtyOutput { session_id, bytes } = event {
            registry.handle_output(session_id, &bytes);
        }
    }
}
