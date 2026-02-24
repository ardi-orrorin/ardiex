use super::FileWatcher;
use notify::event::{AccessKind, CreateKind, EventAttributes, ModifyKind, RemoveKind};
use notify::{Event, EventKind};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc as tokio_mpsc;

fn make_event(kind: EventKind, path: &str) -> Event {
    Event {
        kind,
        paths: vec![PathBuf::from(path)],
        attrs: EventAttributes::default(),
    }
}

#[test]
fn should_trigger_backup_on_create_event() {
    let event = make_event(EventKind::Create(CreateKind::File), "/tmp/a.txt");
    assert!(FileWatcher::should_trigger_backup(&event));
}

#[test]
fn should_trigger_backup_on_remove_event() {
    let event = make_event(EventKind::Remove(RemoveKind::File), "/tmp/a.txt");
    assert!(FileWatcher::should_trigger_backup(&event));
}

#[test]
fn should_trigger_backup_on_regular_modify_event() {
    let event = make_event(EventKind::Modify(ModifyKind::Any), "/tmp/a.txt");
    assert!(FileWatcher::should_trigger_backup(&event));
}

#[test]
fn should_not_trigger_backup_on_temp_modify_event() {
    let tmp_event = make_event(EventKind::Modify(ModifyKind::Any), "/tmp/a.txt.tmp");
    let swp_event = make_event(EventKind::Modify(ModifyKind::Any), "/tmp/a.txt.swp");
    let lock_event = make_event(EventKind::Modify(ModifyKind::Any), "/tmp/a.txt.lock");

    assert!(!FileWatcher::should_trigger_backup(&tmp_event));
    assert!(!FileWatcher::should_trigger_backup(&swp_event));
    assert!(!FileWatcher::should_trigger_backup(&lock_event));
}

#[test]
fn should_not_trigger_backup_on_unrelated_event_kind() {
    let event = make_event(EventKind::Access(AccessKind::Any), "/tmp/a.txt");
    assert!(!FileWatcher::should_trigger_backup(&event));
}

#[tokio::test]
async fn debounce_events_sends_backup_trigger_after_quiet_period() {
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let (backup_tx, mut backup_rx) = tokio_mpsc::channel::<()>(2);

    let handle = std::thread::spawn(move || {
        FileWatcher::debounce_events(event_rx, backup_tx, Duration::from_millis(30));
    });

    event_tx
        .send(make_event(
            EventKind::Create(CreateKind::File),
            "/tmp/trigger.txt",
        ))
        .expect("event send must succeed");
    std::thread::sleep(Duration::from_millis(60));
    drop(event_tx);

    let received = tokio::time::timeout(Duration::from_millis(500), backup_rx.recv())
        .await
        .expect("must receive debounce result within timeout");
    assert!(received.is_some());

    handle.join().expect("debounce thread must finish cleanly");
}

#[tokio::test]
async fn debounce_events_ignores_temp_modify_event() {
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let (backup_tx, mut backup_rx) = tokio_mpsc::channel::<()>(2);

    let handle = std::thread::spawn(move || {
        FileWatcher::debounce_events(event_rx, backup_tx, Duration::from_millis(30));
    });

    event_tx
        .send(make_event(
            EventKind::Modify(ModifyKind::Any),
            "/tmp/ignored.lock",
        ))
        .expect("event send must succeed");
    std::thread::sleep(Duration::from_millis(40));
    drop(event_tx);

    let result = tokio::time::timeout(Duration::from_millis(200), backup_rx.recv())
        .await
        .expect("channel must resolve");
    assert!(
        result.is_none(),
        "ignored temp event must not produce backup trigger"
    );

    handle.join().expect("debounce thread must finish cleanly");
}

#[tokio::test]
async fn debounce_events_coalesces_bursty_events_into_single_trigger() {
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let (backup_tx, mut backup_rx) = tokio_mpsc::channel::<()>(4);

    let handle = std::thread::spawn(move || {
        FileWatcher::debounce_events(event_rx, backup_tx, Duration::from_millis(40));
    });

    event_tx
        .send(make_event(
            EventKind::Create(CreateKind::File),
            "/tmp/a.txt",
        ))
        .expect("event send must succeed");
    std::thread::sleep(Duration::from_millis(10));
    event_tx
        .send(make_event(EventKind::Modify(ModifyKind::Any), "/tmp/a.txt"))
        .expect("event send must succeed");
    std::thread::sleep(Duration::from_millis(10));
    event_tx
        .send(make_event(EventKind::Modify(ModifyKind::Any), "/tmp/a.txt"))
        .expect("event send must succeed");
    std::thread::sleep(Duration::from_millis(70));
    drop(event_tx);

    let first = tokio::time::timeout(Duration::from_millis(500), backup_rx.recv())
        .await
        .expect("must receive first trigger");
    assert!(first.is_some());

    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(backup_rx.try_recv().is_err());

    handle.join().expect("debounce thread must finish cleanly");
}

#[test]
fn should_not_trigger_backup_when_modify_event_contains_temp_path_among_paths() {
    let event = Event {
        kind: EventKind::Modify(ModifyKind::Any),
        paths: vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/a.txt.tmp")],
        attrs: EventAttributes::default(),
    };

    assert!(!FileWatcher::should_trigger_backup(&event));
}

#[tokio::test]
async fn debounce_events_returns_without_trigger_when_sender_disconnected_without_events() {
    let (_event_tx, event_rx) = std::sync::mpsc::channel::<Event>();
    let (backup_tx, mut backup_rx) = tokio_mpsc::channel::<()>(1);

    let handle = std::thread::spawn(move || {
        FileWatcher::debounce_events(event_rx, backup_tx, Duration::from_millis(20));
    });

    // Explicitly disconnect sender without producing any event.
    drop(_event_tx);

    let result = tokio::time::timeout(Duration::from_millis(120), backup_rx.recv())
        .await
        .expect("receiver must resolve after sender disconnect");
    assert!(result.is_none());

    handle.join().expect("debounce thread must finish cleanly");
}
