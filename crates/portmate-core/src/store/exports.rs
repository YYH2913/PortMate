use super::SessionStore;
use crate::redaction::{
    redact_audit_records, redact_session_events, redact_session_summary, redact_sysmon_snapshot,
    redact_timeline_marks, redact_transfer_task,
};

impl SessionStore {
    pub fn export_session_bundle(&self, session_id: &str) -> serde_json::Value {
        self.export_session_bundle_with_redaction(session_id, false)
    }

    pub fn export_session_bundle_redacted(&self, session_id: &str) -> serde_json::Value {
        self.export_session_bundle_with_redaction(session_id, true)
    }

    fn export_session_bundle_with_redaction(
        &self,
        session_id: &str,
        redact_bundle: bool,
    ) -> serde_json::Value {
        let mut summary = self
            .summaries()
            .into_iter()
            .find(|summary| summary.profile.id == session_id);
        let mut events = self.tail_log(session_id, 500);
        let mut timeline = self.timeline_for(session_id);
        let mut transfers = self
            .transfers
            .iter()
            .filter(|transfer| transfer.session_id == session_id)
            .cloned()
            .collect::<Vec<_>>();
        let mut audit = self
            .audit
            .iter()
            .filter(|record| record.session_id.as_deref() == Some(session_id))
            .cloned()
            .collect::<Vec<_>>();
        let mut sysmon = self.sysmon_for(session_id);
        if redact_bundle {
            summary = summary.map(redact_session_summary);
            events = redact_session_events(events);
            timeline = redact_timeline_marks(timeline);
            transfers = transfers.into_iter().map(redact_transfer_task).collect();
            audit = redact_audit_records(audit);
            sysmon = sysmon.map(redact_sysmon_snapshot);
        }
        let log_shards = events
            .iter()
            .filter_map(|event| event.bytes_ref.as_ref())
            .cloned()
            .collect::<Vec<_>>();

        serde_json::json!({
            "summary": summary,
            "events": events,
            "logShards": log_shards,
            "timeline": timeline,
            "sysmon": sysmon,
            "transfers": transfers,
            "audit": audit,
        })
    }
}
