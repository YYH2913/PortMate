impl ModemConnectionWatch {
    #[cfg(test)]
    fn store_only(store: Arc<Mutex<SessionStore>>, session_id: String) -> Self {
        Self {
            store,
            runtimes: None,
            session_id,
            runtime_id: None,
            runtime_kind: None,
        }
    }
}

#[cfg(test)]
pub(super) fn runtime_tap_receiver(
    state: &AppState,
    session_id: &str,
) -> Result<broadcast::Receiver<Vec<u8>>, String> {
    runtime_modem_binding(state, session_id).map(|binding| binding.subscribe())
}

impl ModemByteReader {
    #[cfg(test)]
    pub(super) fn new(receiver: broadcast::Receiver<Vec<u8>>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            receiver,
            pending: VecDeque::new(),
            cancel,
            connection: None,
        }
    }

    #[cfg(test)]
    pub(super) fn watch_connection(
        mut self,
        store: Arc<Mutex<SessionStore>>,
        session_id: String,
    ) -> Self {
        self.connection = Some(ModemConnectionWatch::store_only(store, session_id));
        self
    }

    #[cfg(test)]
    pub(super) async fn after_marker(
        receiver: broadcast::Receiver<Vec<u8>>,
        marker: &str,
        cancel: Arc<AtomicBool>,
        connection: Option<(Arc<Mutex<SessionStore>>, String)>,
    ) -> Result<Self, String> {
        Self::after_marker_with_watch(
            receiver,
            marker,
            cancel,
            connection
                .map(|(store, session_id)| ModemConnectionWatch::store_only(store, session_id)),
        )
        .await
    }
}
