use super::{SessionStore, MAX_EVENTS_PER_SESSION};
use crate::models::{AuditRecord, SysmonSnapshot, TimelineMark, TransferStatus, TransferTask};
use std::collections::{HashMap, HashSet};

pub(super) const MAX_AUDIT_RECORDS_PER_SCOPE: usize = 5000;
pub(super) const MAX_TIMELINE_MARKS_PER_SESSION: usize = 2000;
pub(super) const MAX_SYSMON_SNAPSHOTS_PER_SESSION: usize = 1024;
pub(super) const MAX_TERMINAL_TRANSFERS_PER_SESSION: usize = 1000;
pub(super) const AUX_HISTORY_TRIM_BATCH: usize = 128;

impl SessionStore {
    pub fn transfer_by_id(&self, id: &str) -> Option<TransferTask> {
        self.transfers
            .iter()
            .find(|transfer| transfer.id == id)
            .cloned()
    }

    pub fn record_transfer(&mut self, transfer: TransferTask) {
        let session_id = transfer.session_id.clone();
        self.transfers.push(transfer);
        self.trim_transfer_history(&session_id);
    }

    pub fn trim_transfer_history(&mut self, session_id: &str) {
        let mut terminal = self
            .transfers
            .iter()
            .enumerate()
            .filter(|(_, transfer)| {
                transfer.session_id == session_id
                    && matches!(
                        transfer.status,
                        TransferStatus::Completed
                            | TransferStatus::Failed
                            | TransferStatus::Cancelled
                    )
            })
            .map(|(index, transfer)| (index, transfer.finished_at))
            .collect::<Vec<_>>();
        if terminal.len() <= MAX_TERMINAL_TRANSFERS_PER_SESSION {
            return;
        }
        let to_drop = terminal.len() - MAX_TERMINAL_TRANSFERS_PER_SESSION;
        terminal.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        let remove = terminal
            .into_iter()
            .take(to_drop)
            .map(|(index, _)| index)
            .collect::<HashSet<_>>();
        let mut index = 0_usize;
        self.transfers.retain(|_| {
            let keep = !remove.contains(&index);
            index += 1;
            keep
        });
    }

    pub fn record_audit(&mut self, record: AuditRecord) {
        let scope = record.session_id.clone();
        self.audit.push(record);
        trim_oldest_matching(
            &mut self.audit,
            MAX_AUDIT_RECORDS_PER_SCOPE,
            AUX_HISTORY_TRIM_BATCH,
            |record| record.session_id == scope,
        );
    }

    pub fn record_timeline_mark(&mut self, mark: TimelineMark) {
        let session_id = mark.session_id.clone();
        self.timeline.push(mark);
        trim_oldest_matching(
            &mut self.timeline,
            MAX_TIMELINE_MARKS_PER_SESSION,
            AUX_HISTORY_TRIM_BATCH,
            |mark| mark.session_id == session_id,
        );
    }

    pub fn record_sysmon_snapshot(&mut self, snapshot: SysmonSnapshot) {
        let session_id = snapshot.session_id.clone();
        self.sysmon.push(snapshot);
        trim_oldest_matching(
            &mut self.sysmon,
            MAX_SYSMON_SNAPSHOTS_PER_SESSION,
            AUX_HISTORY_TRIM_BATCH,
            |snapshot| snapshot.session_id == session_id,
        );
    }

    pub fn normalize_bounded_histories(&mut self) {
        let mut remaining_event_counts = HashMap::<String, usize>::new();
        for event in &self.events {
            *remaining_event_counts
                .entry(event.session_id.clone())
                .or_default() += 1;
        }
        self.events.retain(|event| {
            let remaining = remaining_event_counts
                .get_mut(&event.session_id)
                .expect("event session count was seeded");
            let keep = *remaining <= MAX_EVENTS_PER_SESSION;
            *remaining -= 1;
            keep
        });
        self.event_counts.clear();
        for event in &self.events {
            *self
                .event_counts
                .entry(event.session_id.clone())
                .or_default() += 1;
        }

        let audit_scopes = self
            .audit
            .iter()
            .map(|record| record.session_id.clone())
            .collect::<HashSet<_>>();
        for scope in audit_scopes {
            trim_oldest_matching(&mut self.audit, MAX_AUDIT_RECORDS_PER_SCOPE, 0, |record| {
                record.session_id == scope
            });
        }

        let timeline_sessions = self
            .timeline
            .iter()
            .map(|mark| mark.session_id.clone())
            .collect::<HashSet<_>>();
        for session_id in timeline_sessions {
            trim_oldest_matching(
                &mut self.timeline,
                MAX_TIMELINE_MARKS_PER_SESSION,
                0,
                |mark| mark.session_id == session_id,
            );
        }

        let sysmon_sessions = self
            .sysmon
            .iter()
            .map(|snapshot| snapshot.session_id.clone())
            .collect::<HashSet<_>>();
        for session_id in sysmon_sessions {
            trim_oldest_matching(
                &mut self.sysmon,
                MAX_SYSMON_SNAPSHOTS_PER_SESSION,
                0,
                |snapshot| snapshot.session_id == session_id,
            );
        }

        let transfer_sessions = self
            .transfers
            .iter()
            .map(|transfer| transfer.session_id.clone())
            .collect::<HashSet<_>>();
        for session_id in transfer_sessions {
            self.trim_transfer_history(&session_id);
        }
    }

    pub fn sysmon_for(&self, session_id: &str) -> Option<SysmonSnapshot> {
        self.sysmon
            .iter()
            .rev()
            .find(|snapshot| snapshot.session_id == session_id)
            .cloned()
    }

    pub fn sysmon_history_for(&self, session_id: &str, limit: usize) -> Vec<SysmonSnapshot> {
        if limit == 0 {
            return Vec::new();
        }
        let mut snapshots = self
            .sysmon
            .iter()
            .rev()
            .filter(|snapshot| snapshot.session_id == session_id)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        snapshots.reverse();
        snapshots
    }

    pub fn timeline_for(&self, session_id: &str) -> Vec<TimelineMark> {
        self.timeline
            .iter()
            .filter(|mark| mark.session_id == session_id)
            .cloned()
            .collect()
    }
}

fn trim_oldest_matching<T>(
    items: &mut Vec<T>,
    max: usize,
    slack: usize,
    mut matches: impl FnMut(&T) -> bool,
) {
    let count = items.iter().filter(|item| matches(item)).count();
    if count <= max.saturating_add(slack) {
        return;
    }
    let mut to_drop = count - max;
    items.retain(|item| {
        if to_drop > 0 && matches(item) {
            to_drop -= 1;
            false
        } else {
            true
        }
    });
}
