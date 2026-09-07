//! Window-owned sender jobs pin all targets before the first repeat. Removing
//! a job cancels queued payloads and makes late IPC fail closed.
use super::*;

type JobKey = (PathBuf, String, String);
static JOBS: OnceLock<Mutex<HashMap<JobKey, Arc<PacedSendJob>>>> = OnceLock::new();

pub(super) struct PacedSendJob {
    runtimes: HashMap<String, String>,
    pub(super) cancelled: Arc<AtomicBool>,
    changed: tokio::sync::Notify,
}

impl PacedSendJob {
    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.changed.notify_waiters();
    }

    pub(super) async fn wait_cancelled(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.cancelled.load(Ordering::SeqCst) {
                return;
            }
            changed.await;
        }
    }
}

pub(super) fn begin_job(
    io: &SessionIo,
    owner: &str,
    session_ids: Vec<String>,
) -> Result<String, String> {
    if session_ids.is_empty() || session_ids.len() > 1024 {
        return Err("发送目标数量必须为 1–1024".into());
    }
    let mut runtimes = HashMap::new();
    for id in session_ids {
        let runtime = current_session_runtime_id(&io.runtimes, &id)?
            .ok_or_else(|| "发送目标尚未连接".to_string())?;
        runtimes.insert(id, runtime);
    }
    let mut jobs = JOBS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|e| e.to_string())?;
    if jobs.len() >= 128 {
        return Err("间隔发送任务数量已达上限".into());
    }
    let id = Uuid::new_v4().to_string();
    jobs.insert(
        (io.store_path.clone(), owner.to_string(), id.clone()),
        Arc::new(PacedSendJob {
            runtimes,
            cancelled: Arc::new(AtomicBool::new(false)),
            changed: tokio::sync::Notify::new(),
        }),
    );
    Ok(id)
}

pub(super) fn target(
    io: &SessionIo,
    owner: &str,
    job_id: &str,
    session_id: &str,
) -> Result<(Arc<PacedSendJob>, String), String> {
    let job = JOBS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|e| e.to_string())?
        .get(&(io.store_path.clone(), owner.to_string(), job_id.to_string()))
        .cloned()
        .ok_or_else(|| "间隔发送任务已结束或已取消".to_string())?;
    if job.cancelled.load(Ordering::SeqCst) {
        return Err("间隔发送已取消".into());
    }
    let runtime = job
        .runtimes
        .get(session_id)
        .cloned()
        .ok_or_else(|| "会话不属于此发送任务".to_string())?;
    if current_session_runtime_id(&io.runtimes, session_id)?.as_deref() != Some(&runtime) {
        job.cancel();
        return Err("连接已变化，间隔发送已停止，请重新启动发送任务".into());
    }
    Ok((job, runtime))
}

pub(super) fn cancel_job(path: &Path, owner: &str, job_id: &str) {
    if let Some(jobs) = JOBS.get() {
        if let Some(job) = jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(path.to_path_buf(), owner.to_string(), job_id.to_string()))
        {
            job.cancel();
        }
    }
}

pub(super) fn clear_owner(path: &Path, owner: &str) {
    if let Some(jobs) = JOBS.get() {
        jobs.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(p, o, _), job| {
                if p == path && o == owner {
                    job.cancel();
                    false
                } else {
                    true
                }
            });
    }
}

pub(super) fn clear_session(path: &Path, session_id: &str) {
    if let Some(jobs) = JOBS.get() {
        jobs.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(p, _, _), job| {
                if p == path && job.runtimes.contains_key(session_id) {
                    job.cancel();
                    false
                } else {
                    true
                }
            });
    }
}

#[tauri::command]
pub(crate) fn begin_paced_send(
    state: State<'_, AppState>,
    window: WebviewWindow,
    session_ids: Vec<String>,
) -> Result<String, String> {
    begin_job(&state.session_io(), window.label(), session_ids)
}

#[tauri::command]
pub(crate) fn cancel_paced_send(state: State<'_, AppState>, window: WebviewWindow, job_id: String) {
    cancel_job(&state.store_path, window.label(), &job_id);
}
