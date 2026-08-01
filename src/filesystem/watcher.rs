use std::{path::PathBuf, sync::mpsc, thread, time::Duration};

use anyhow::{Context, Result};
use notify::{event::ModifyKind, Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::event::{AppEvent, EventSender, FsEventBatch};

pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(150);

pub struct WatchHandle {
    _watcher: RecommendedWatcher,
    _thread: thread::JoinHandle<()>,
}

pub fn start(root: PathBuf, event_tx: EventSender, debounce: Duration) -> Result<WatchHandle> {
    let (raw_tx, raw_rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |result: notify::Result<notify::Event>| {
            if let Ok(event) = result {
                let _ = raw_tx.send(event);
            }
        },
        Config::default(),
    )
    .context("Unable to initialize filesystem watcher")?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("Unable to watch {}", root.display()))?;

    let thread = thread::spawn(move || {
        while let Ok(event) = raw_rx.recv() {
            let mut paths = event.paths;
            let mut tree_changed = event_requires_tree_refresh(&event.kind);
            loop {
                match raw_rx.recv_timeout(debounce) {
                    Ok(event) => {
                        tree_changed |= event_requires_tree_refresh(&event.kind);
                        paths.extend(event.paths);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        paths.sort();
                        paths.dedup();
                        let _ = event_tx.send(AppEvent::FileSystem(FsEventBatch {
                            paths,
                            tree_changed,
                        }));
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        }
    });

    Ok(WatchHandle {
        _watcher: watcher,
        _thread: thread,
    })
}

fn event_requires_tree_refresh(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Name(_))
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Any
    )
}
