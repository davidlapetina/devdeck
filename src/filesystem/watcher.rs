use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use notify::{
    event::{ModifyKind, RenameMode},
    Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};

use crate::event::{AppEvent, EventSender, FsEventBatch};

pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(150);
pub const MAX_BATCH_AGE: Duration = Duration::from_millis(500);
pub const RAW_EVENT_CHANNEL_CAPACITY: usize = 256;
pub const MAX_BATCH_PATHS: usize = 512;
pub const MAX_BATCH_RENAMES: usize = 128;

pub struct WatchHandle {
    _watcher: RecommendedWatcher,
    _thread: thread::JoinHandle<()>,
}

pub fn start(root: PathBuf, event_tx: EventSender, debounce: Duration) -> Result<WatchHandle> {
    let (raw_tx, raw_rx) = mpsc::sync_channel(RAW_EVENT_CHANNEL_CAPACITY);
    let raw_overflowed = Arc::new(AtomicBool::new(false));
    let callback_overflowed = Arc::clone(&raw_overflowed);
    let mut watcher = RecommendedWatcher::new(
        move |result: notify::Result<notify::Event>| {
            if let Ok(event) = result {
                if matches!(raw_tx.try_send(event), Err(mpsc::TrySendError::Full(_))) {
                    callback_overflowed.store(true, Ordering::Release);
                }
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
            let started = Instant::now();
            let mut batch = PendingBatch::default();
            batch.push(event);
            loop {
                if raw_overflowed.swap(false, Ordering::AcqRel) {
                    batch.overflow();
                }
                let Some(wait) = next_batch_wait(started.elapsed(), debounce) else {
                    let _ = event_tx.send(AppEvent::FileSystem(batch.finish()));
                    break;
                };
                match raw_rx.recv_timeout(wait) {
                    Ok(event) => {
                        batch.push(event);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if raw_overflowed.swap(false, Ordering::AcqRel) {
                            batch.overflow();
                        }
                        let _ = event_tx.send(AppEvent::FileSystem(batch.finish()));
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

fn next_batch_wait(elapsed: Duration, debounce: Duration) -> Option<Duration> {
    let remaining = MAX_BATCH_AGE.checked_sub(elapsed)?;
    (!remaining.is_zero()).then(|| debounce.min(remaining))
}

#[derive(Default)]
struct PendingBatch {
    paths: Vec<PathBuf>,
    tree_changed: bool,
    renames: Vec<(PathBuf, PathBuf)>,
    pending_rename_sources: VecDeque<PathBuf>,
    overflowed: bool,
}

impl PendingBatch {
    fn push(&mut self, event: notify::Event) {
        if self.overflowed {
            return;
        }
        if self.paths.len().saturating_add(event.paths.len()) > MAX_BATCH_PATHS
            || rename_would_overflow(
                &event.kind,
                event.paths.len(),
                self.pending_rename_sources.len(),
                self.renames.len(),
            )
        {
            self.overflow();
            return;
        }
        self.tree_changed |= event_requires_tree_refresh(&event.kind);
        record_renames(
            &event.kind,
            &event.paths,
            &mut self.pending_rename_sources,
            &mut self.renames,
        );
        self.paths.extend(event.paths);
    }

    fn overflow(&mut self) {
        self.paths.clear();
        self.renames.clear();
        self.pending_rename_sources.clear();
        self.tree_changed = true;
        self.overflowed = true;
    }

    fn finish(mut self) -> FsEventBatch {
        self.paths.sort();
        self.paths.dedup();
        FsEventBatch {
            paths: self.paths,
            tree_changed: self.tree_changed,
            renames: self.renames,
        }
    }
}

fn rename_would_overflow(
    kind: &EventKind,
    path_count: usize,
    pending_count: usize,
    rename_count: usize,
) -> bool {
    match kind {
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            rename_count.saturating_add(path_count / 2) > MAX_BATCH_RENAMES
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            pending_count.saturating_add(path_count) > MAX_BATCH_RENAMES
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            rename_count.saturating_add(path_count.min(pending_count)) > MAX_BATCH_RENAMES
        }
        _ => false,
    }
}

fn record_renames(
    kind: &EventKind,
    paths: &[PathBuf],
    pending_sources: &mut VecDeque<PathBuf>,
    renames: &mut Vec<(PathBuf, PathBuf)>,
) {
    match kind {
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if paths.len() >= 2 => {
            for pair in paths.chunks_exact(2) {
                renames.push((pair[0].clone(), pair[1].clone()));
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            pending_sources.extend(paths.iter().cloned());
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            for destination in paths {
                if let Some(source) = pending_sources.pop_front() {
                    renames.push((source, destination.clone()));
                }
            }
        }
        _ => {}
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_combined_and_split_rename_notifications() {
        let mut pending = VecDeque::new();
        let mut renames = Vec::new();
        record_renames(
            &EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &[PathBuf::from("old"), PathBuf::from("new")],
            &mut pending,
            &mut renames,
        );
        record_renames(
            &EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            &[PathBuf::from("from")],
            &mut pending,
            &mut renames,
        );
        record_renames(
            &EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            &[PathBuf::from("to")],
            &mut pending,
            &mut renames,
        );

        assert_eq!(
            renames,
            vec![
                (PathBuf::from("old"), PathBuf::from("new")),
                (PathBuf::from("from"), PathBuf::from("to")),
            ]
        );
    }

    #[test]
    fn unmatched_rename_destination_is_a_safe_noop() {
        let mut pending = VecDeque::new();
        let mut renames = Vec::new();
        record_renames(
            &EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            &[PathBuf::from("to")],
            &mut pending,
            &mut renames,
        );
        assert!(renames.is_empty());
    }

    #[test]
    fn oversized_path_batch_coalesces_to_full_refresh() {
        let mut batch = PendingBatch::default();
        let mut event = notify::Event::new(EventKind::Modify(ModifyKind::Any));
        event.paths = (0..=MAX_BATCH_PATHS)
            .map(|index| PathBuf::from(format!("path-{index}")))
            .collect();

        batch.push(event);
        let finished = batch.finish();

        assert!(finished.tree_changed);
        assert!(finished.paths.is_empty());
        assert!(finished.renames.is_empty());
    }

    #[test]
    fn rename_overflow_coalesces_to_full_refresh() {
        let mut batch = PendingBatch::default();
        for index in 0..=MAX_BATCH_RENAMES {
            let mut event =
                notify::Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)));
            event.paths = vec![
                PathBuf::from(format!("old-{index}")),
                PathBuf::from(format!("new-{index}")),
            ];
            batch.push(event);
        }

        let finished = batch.finish();
        assert!(finished.tree_changed);
        assert!(finished.paths.is_empty());
        assert!(finished.renames.is_empty());
    }

    #[test]
    fn max_batch_age_forces_flush_during_sustained_events() {
        assert_eq!(
            next_batch_wait(Duration::ZERO, DEFAULT_DEBOUNCE),
            Some(DEFAULT_DEBOUNCE)
        );
        assert_eq!(
            next_batch_wait(MAX_BATCH_AGE - Duration::from_millis(10), DEFAULT_DEBOUNCE),
            Some(Duration::from_millis(10))
        );
        assert_eq!(next_batch_wait(MAX_BATCH_AGE, DEFAULT_DEBOUNCE), None);
        assert_eq!(
            next_batch_wait(MAX_BATCH_AGE + Duration::from_millis(1), DEFAULT_DEBOUNCE),
            None
        );
    }
}
