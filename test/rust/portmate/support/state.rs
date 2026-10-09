impl SerialWorkerRegistry {
    #[cfg(test)]
    pub(super) fn register(self: &Arc<Self>) -> Result<SerialWorkerGuard, String> {
        self.register_inner(None)
    }
}
