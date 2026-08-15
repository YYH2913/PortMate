use super::events::MAX_EVENTS_PER_SESSION;
use super::SessionStore;
use crate::models::{
    AuditRecord, CommandHistoryEntry, SysmonSnapshot, TimelineMark, TransferStatus, TransferTask,
};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

pub(super) const MAX_AUDIT_RECORDS_PER_SCOPE: usize = 5000;
pub(super) const MAX_TIMELINE_MARKS_PER_SESSION: usize = 2000;
pub(super) const MAX_SYSMON_SNAPSHOTS_PER_SESSION: usize = 1024;
pub(super) const MAX_TERMINAL_TRANSFERS_PER_SESSION: usize = 1000;
pub(super) const AUX_HISTORY_TRIM_BATCH: usize = 128;
pub const MAX_COMMAND_HISTORY_ENTRIES: usize = 10_000;
pub const MAX_COMMAND_HISTORY_RETENTION_DAYS: u32 = 3_650;
pub const MAX_COMMAND_HISTORY_COMMAND_CHARACTERS: usize = 8_192;
pub const MAX_COMMAND_HISTORY_STORAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_COMMAND_HISTORY_INPUT_ENTRIES: usize = MAX_COMMAND_HISTORY_ENTRIES * 2;
const COMMAND_HISTORY_EMPTY_SNAPSHOT_BYTES: usize = b"{\"version\":2,\"entries\":[]}".len();
const MAX_JAVASCRIPT_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

impl SessionStore {
    pub fn normalized_command_history(
        entries: &[CommandHistoryEntry],
        limit: usize,
        retention_days: u32,
        now_ms: i64,
    ) -> Result<Vec<CommandHistoryEntry>, String> {
        if !(1..=MAX_COMMAND_HISTORY_ENTRIES).contains(&limit) {
            return Err(format!(
                "command history limit must be between 1 and {MAX_COMMAND_HISTORY_ENTRIES}"
            ));
        }
        if retention_days > MAX_COMMAND_HISTORY_RETENTION_DAYS {
            return Err(format!(
                "command history retention must be between 0 and {MAX_COMMAND_HISTORY_RETENTION_DAYS} days"
            ));
        }
        let retention_ms = i64::from(retention_days).saturating_mul(24 * 60 * 60 * 1_000);
        let cutoff = if retention_days == 0 {
            i64::MIN
        } else {
            now_ms.saturating_sub(retention_ms)
        };
        let mut seen = HashSet::new();
        let mut normalized = Vec::new();
        let mut bytes = COMMAND_HISTORY_EMPTY_SNAPSHOT_BYTES;
        for entry in entries.iter().take(MAX_COMMAND_HISTORY_INPUT_ENTRIES) {
            if entry.command.trim().is_empty()
                || entry.command.contains('\0')
                || entry.command.chars().count() > MAX_COMMAND_HISTORY_COMMAND_CHARACTERS
                || seen.contains(&entry.command)
            {
                continue;
            }
            let recorded_at = entry.recorded_at.clamp(0, now_ms.max(0));
            if recorded_at < cutoff {
                continue;
            }
            let candidate = CommandHistoryEntry {
                command: entry.command.clone(),
                recorded_at,
            };
            let entry_bytes = serde_json::to_vec(&candidate)
                .map_err(|error| format!("failed to size command history entry: {error}"))?
                .len()
                .saturating_add(usize::from(!normalized.is_empty()));
            if bytes.saturating_add(entry_bytes) > MAX_COMMAND_HISTORY_STORAGE_BYTES {
                continue;
            }
            bytes += entry_bytes;
            seen.insert(candidate.command.clone());
            normalized.push(candidate);
            if normalized.len() >= limit {
                break;
            }
        }
        Ok(normalized)
    }

    pub fn replace_command_history(
        &mut self,
        entries: &[CommandHistoryEntry],
        limit: usize,
        retention_days: u32,
        now_ms: i64,
    ) -> Result<Vec<CommandHistoryEntry>, String> {
        let normalized = Self::normalized_command_history(entries, limit, retention_days, now_ms)?;
        let changed = self.command_history != normalized || !self.command_history_migrated;
        if changed && self.command_history_revision >= MAX_JAVASCRIPT_SAFE_INTEGER {
            return Err(
                "command history revision is exhausted; restart with a repaired Store".to_string(),
            );
        }
        self.command_history.clone_from(&normalized);
        self.command_history_migrated = true;
        if changed {
            self.command_history_revision += 1;
        }
        Ok(normalized)
    }

    pub fn merge_command_history(
        &mut self,
        entries: &[CommandHistoryEntry],
        limit: usize,
        retention_days: u32,
        now_ms: i64,
    ) -> Result<Vec<CommandHistoryEntry>, String> {
        let mut merged =
            Vec::with_capacity(entries.len().saturating_add(self.command_history.len()));
        merged.extend_from_slice(entries);
        merged.extend(self.command_history.iter().cloned());
        merged.sort_by_key(|entry| Reverse(entry.recorded_at));
        self.replace_command_history(&merged, limit, retention_days, now_ms)
    }

    pub fn record_command_history(
        &mut self,
        command: String,
        limit: usize,
        retention_days: u32,
        now_ms: i64,
    ) -> Result<Vec<CommandHistoryEntry>, String> {
        let candidate = Self::normalized_command_history(
            &[CommandHistoryEntry {
                command: command.clone(),
                recorded_at: now_ms,
            }],
            1,
            0,
            now_ms,
        )?;
        if candidate.is_empty() {
            return Err(
                "command history entry is empty, invalid, or exceeds its character limit"
                    .to_string(),
            );
        }
        let recorded_command = command.clone();
        let mut entries = Vec::with_capacity(self.command_history.len().saturating_add(1));
        entries.push(CommandHistoryEntry {
            command,
            recorded_at: now_ms,
        });
        entries.extend(
            self.command_history
                .iter()
                .filter(|entry| entry.command != recorded_command)
                .cloned(),
        );
        self.replace_command_history(&entries, limit, retention_days, now_ms)
    }

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
        self.command_history_revision = self
            .command_history_revision
            .min(MAX_JAVASCRIPT_SAFE_INTEGER.saturating_sub(1));
        if let Ok(normalized) = Self::normalized_command_history(
            &self.command_history,
            MAX_COMMAND_HISTORY_ENTRIES,
            0,
            chrono::Utc::now().timestamp_millis(),
        ) {
            self.command_history = normalized;
        }
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
