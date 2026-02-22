use anyhow::Result;
use log::{error, info, warn};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc as tokio_mpsc;

pub struct FileWatcher {
    _watchers: Vec<RecommendedWatcher>,
    _backup_tx: tokio_mpsc::Sender<()>,
    _debounce_duration: Duration,
}

impl FileWatcher {
    pub fn new(
        watch_paths: Vec<PathBuf>,
        backup_tx: tokio_mpsc::Sender<()>,
        debounce_duration: Duration,
    ) -> Result<Self> {
        let mut watchers = Vec::new();

        for path in watch_paths {
            if !path.exists() {
                warn!("Watch path does not exist: {:?}", path);
                continue;
            }

            let (tx, rx) = mpsc::channel();
            let mut watcher = RecommendedWatcher::new(
                move |res: Result<Event, notify::Error>| match res {
                    Ok(event) => {
                        if let Err(e) = tx.send(event) {
                            error!("Failed to send file system event: {}", e);
                        }
                    }
                    Err(e) => error!("File system watch error: {:?}", e),
                },
                Config::default(),
            )?;

            watcher.watch(&path, RecursiveMode::Recursive)?;
            watchers.push(watcher);
            info!("Started watching: {:?}", path);
            let backup_tx_clone = backup_tx.clone();
            let debounce = debounce_duration;

            thread::spawn(move || {
                Self::debounce_events(rx, backup_tx_clone, debounce);
            });
        }

        Ok(Self {
            _watchers: watchers,
            _backup_tx: backup_tx,
            _debounce_duration: debounce_duration,
        })
    }

    fn debounce_events(
        rx: mpsc::Receiver<Event>,
        backup_tx: tokio_mpsc::Sender<()>,
        debounce_duration: Duration,
    ) {
        let mut last_event_time;
        let mut pending_backup;

        while let Ok(event) = rx.recv() {
            if Self::should_trigger_backup(&event) {
                let now = std::time::Instant::now();
                last_event_time = now;
                pending_backup = true;

                while pending_backup {
                    match rx.recv_timeout(debounce_duration) {
                        Ok(event) => {
                            if Self::should_trigger_backup(&event) {
                                last_event_time = std::time::Instant::now();
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if last_event_time.elapsed() >= debounce_duration {
                                if let Err(e) = backup_tx.blocking_send(()) {
                                    error!("Failed to send backup trigger: {}", e);
                                    break;
                                }
                                info!("File changes detected, triggering backup");
                                pending_backup = false;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            error!("Event channel disconnected");
                            return;
                        }
                    }
                }
            }
        }
    }

    fn should_trigger_backup(event: &Event) -> bool {
        match &event.kind {
            EventKind::Create(_) => true,
            EventKind::Modify(_) => !event.paths.iter().any(|p| {
                if let Some(name) = p.file_name() {
                    let name_str = name.to_string_lossy();
                    name_str.ends_with(".tmp")
                        || name_str.ends_with(".swp")
                        || name_str.ends_with(".lock")
                } else {
                    false
                }
            }),
            EventKind::Remove(_) => true,
            _ => false,
        }
    }
}
