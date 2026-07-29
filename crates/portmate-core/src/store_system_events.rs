use crate::models::{SessionEvent, SessionProfile};
use std::collections::VecDeque;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

pub(crate) const MAX_SYSTEM_EVENT_OUTBOX: usize = 4096;

type SystemEventEnvelope = (SessionEvent, Option<SessionProfile>);

#[derive(Debug, Default)]
enum SystemEventSinkStatus {
    #[default]
    Inactive,
    Active(SyncSender<()>),
    Failed(String),
}

#[derive(Debug, Default)]
struct SystemEventSinkState {
    status: SystemEventSinkStatus,
    outbox: VecDeque<SystemEventEnvelope>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SystemEventSinkRuntime {
    state: Arc<Mutex<SystemEventSinkState>>,
}

impl SystemEventSinkRuntime {
    pub(crate) fn set_notifier(&self, sender: SyncSender<()>) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "system event sink state poisoned".to_string())?;
        state.status = SystemEventSinkStatus::Active(sender.clone());
        if !state.outbox.is_empty() {
            if let Err(TrySendError::Disconnected(())) = sender.try_send(()) {
                let error = "system event sink worker disconnected".to_string();
                state.status = SystemEventSinkStatus::Failed(error.clone());
                return Err(error);
            }
        }
        Ok(())
    }

    pub(crate) fn clear_notifier(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.status = SystemEventSinkStatus::Inactive;
        }
    }

    pub(crate) fn enqueue(
        &self,
        event: SessionEvent,
        profile: Option<SessionProfile>,
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "system event sink state poisoned".to_string())?;
        let notifier = match &state.status {
            SystemEventSinkStatus::Inactive => return Ok(()),
            SystemEventSinkStatus::Active(notifier) => notifier.clone(),
            SystemEventSinkStatus::Failed(error) => return Err(error.clone()),
        };
        if state.outbox.len() >= MAX_SYSTEM_EVENT_OUTBOX {
            return Err(format!(
                "system event sink backlog exceeded {MAX_SYSTEM_EVENT_OUTBOX} events"
            ));
        }
        state.outbox.push_back((event, profile));
        match notifier.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => Ok(()),
            Err(TrySendError::Disconnected(())) => {
                state.outbox.pop_back();
                let error = "system event sink worker disconnected".to_string();
                state.status = SystemEventSinkStatus::Failed(error.clone());
                Err(error)
            }
        }
    }

    pub(crate) fn drain(&self) -> Vec<SystemEventEnvelope> {
        self.state
            .lock()
            .map(|mut state| state.outbox.drain(..).collect())
            .unwrap_or_default()
    }

    pub(crate) fn discard_session(&self, session_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .outbox
            .retain(|(event, _)| event.session_id != session_id);
    }

    pub(crate) fn discard_event(&self, event_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.outbox.retain(|(event, _)| event.id != event_id);
    }
}
