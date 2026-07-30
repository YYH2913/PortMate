use portmate_core::SessionStore;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{
    app_data_migration::LEGACY_JSON_STORE_FILE_NAME, store_persistence::write_private_atomic_file,
};

const MAX_PENDING_COMPATIBILITY_SNAPSHOTS: usize = 64;

#[derive(Default)]
struct CompatibilitySnapshotState {
    pending: HashMap<PathBuf, SessionStore>,
    order: VecDeque<PathBuf>,
    writing: HashSet<PathBuf>,
    worker_started: bool,
}

struct CompatibilitySnapshotQueue {
    state: Mutex<CompatibilitySnapshotState>,
    changed: Condvar,
}

static COMPATIBILITY_SNAPSHOT_QUEUE: OnceLock<CompatibilitySnapshotQueue> = OnceLock::new();

fn compatibility_snapshot_queue() -> &'static CompatibilitySnapshotQueue {
    COMPATIBILITY_SNAPSHOT_QUEUE.get_or_init(|| CompatibilitySnapshotQueue {
        state: Mutex::new(CompatibilitySnapshotState::default()),
        changed: Condvar::new(),
    })
}

fn compatibility_snapshot_path(store_path: &Path) -> PathBuf {
    store_path.with_file_name(LEGACY_JSON_STORE_FILE_NAME)
}

pub(super) fn enqueue_json_compatibility_snapshot(
    store_path: &Path,
    store: &SessionStore,
) -> Result<(), String> {
    let queue = compatibility_snapshot_queue();
    let snapshot_path = compatibility_snapshot_path(store_path);
    let mut state = queue.state.lock().map_err(|error| error.to_string())?;
    let already_pending = state.pending.contains_key(&snapshot_path);
    if !already_pending && state.pending.len() >= MAX_PENDING_COMPATIBILITY_SNAPSHOTS {
        return Err(format!(
            "JSON compatibility snapshot queue is full ({MAX_PENDING_COMPATIBILITY_SNAPSHOTS})"
        ));
    }
    state.pending.insert(snapshot_path.clone(), store.clone());
    if !already_pending {
        state.order.push_back(snapshot_path);
    }
    if !state.worker_started {
        state.worker_started = true;
        if let Err(error) = std::thread::Builder::new()
            .name("portmate-json-compatibility".to_string())
            .spawn(compatibility_snapshot_worker)
        {
            state.worker_started = false;
            return Err(format!(
                "failed to start JSON compatibility snapshot worker: {error}"
            ));
        }
    }
    queue.changed.notify_one();
    Ok(())
}

fn compatibility_snapshot_worker() {
    let queue = compatibility_snapshot_queue();
    loop {
        let (path, store) = {
            let mut state = queue
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while state.order.is_empty() {
                state = queue
                    .changed
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            let path = state
                .order
                .pop_front()
                .expect("pending compatibility snapshot disappeared");
            let store = state
                .pending
                .remove(&path)
                .expect("pending compatibility snapshot disappeared");
            state.writing.insert(path.clone());
            (path, store)
        };

        if let Err(error) = save_store_json(&path, &store) {
            eprintln!("PortMate: failed to update JSON compatibility store: {error}");
        }

        let mut state = queue
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.writing.remove(&path);
        queue.changed.notify_all();
    }
}

pub(super) fn schedule_json_compatibility_snapshot(store_path: &Path, store: &SessionStore) {
    match enqueue_json_compatibility_snapshot(store_path, store) {
        Ok(()) => {
            #[cfg(test)]
            if let Err(error) =
                flush_json_compatibility_snapshot(store_path, Duration::from_secs(5))
            {
                panic!("test JSON compatibility snapshot did not flush: {error}");
            }
        }
        Err(error) => eprintln!("PortMate: failed to queue JSON compatibility store: {error}"),
    }
}

#[cfg(test)]
pub(super) fn flush_json_compatibility_snapshot(
    store_path: &Path,
    timeout: Duration,
) -> Result<(), String> {
    let snapshot_path = compatibility_snapshot_path(store_path);
    wait_for_compatibility_snapshots(timeout, |state| {
        !state.pending.contains_key(&snapshot_path) && !state.writing.contains(&snapshot_path)
    })
}

pub(super) fn flush_json_compatibility_snapshots(timeout: Duration) -> Result<(), String> {
    wait_for_compatibility_snapshots(timeout, |state| {
        state.pending.is_empty() && state.writing.is_empty()
    })
}

fn wait_for_compatibility_snapshots(
    timeout: Duration,
    finished: impl Fn(&CompatibilitySnapshotState) -> bool,
) -> Result<(), String> {
    let queue = compatibility_snapshot_queue();
    let started = Instant::now();
    let mut state = queue.state.lock().map_err(|error| error.to_string())?;
    while !finished(&state) {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(format!(
                "JSON compatibility snapshot flush exceeded {} ms",
                timeout.as_millis()
            ));
        }
        let (next_state, wait_result) = queue
            .changed
            .wait_timeout(state, remaining)
            .map_err(|error| error.to_string())?;
        state = next_state;
        if wait_result.timed_out() && !finished(&state) {
            return Err(format!(
                "JSON compatibility snapshot flush exceeded {} ms",
                timeout.as_millis()
            ));
        }
    }
    Ok(())
}

pub(super) fn save_store_json(path: &Path, store: &SessionStore) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(store)
        .map_err(|error| format!("failed to serialize PortMate store: {error}"))?;
    write_private_atomic_file(path, &bytes, "PortMate JSON compatibility store")
}
