use super::*;

pub(crate) fn normalize_loaded_mirror_keys(store: &mut SessionStore) {
    normalize_loaded_record_ids(
        &mut store.events,
        "event",
        |event| &event.id,
        |event, id| event.id = id,
    );
    normalize_loaded_record_ids(
        &mut store.transfers,
        "transfer",
        |transfer| &transfer.id,
        |transfer, id| transfer.id = id,
    );
    normalize_loaded_record_ids(
        &mut store.audit,
        "audit",
        |record| &record.id,
        |record, id| record.id = id,
    );
    normalize_loaded_record_ids(
        &mut store.timeline,
        "timeline",
        |mark| &mark.id,
        |mark, id| mark.id = id,
    );
    normalize_loaded_record_ids(
        &mut store.host_keys.keys,
        "host-key",
        |key| &key.id,
        |key, id| key.id = id,
    );

    // The SQLite mirror uses (session_id, ts) as the Sysmon key. A later entry
    // is also what sysmon_for observes for a duplicate timestamp, so retain it.
    let snapshots = std::mem::take(&mut store.sysmon);
    let mut normalized = Vec::with_capacity(snapshots.len());
    let mut seen = HashSet::with_capacity(snapshots.len());
    for snapshot in snapshots.into_iter().rev() {
        let key = (snapshot.session_id.clone(), snapshot.ts.to_rfc3339());
        if seen.insert(key) {
            normalized.push(snapshot);
        }
    }
    normalized.reverse();
    store.sysmon = normalized;
}

pub(crate) fn normalize_loaded_record_ids<T>(
    records: &mut [T],
    record_kind: &str,
    id: impl Fn(&T) -> &str,
    set_id: impl Fn(&mut T, String),
) {
    let reserved_ids = records
        .iter()
        .map(&id)
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned)
        .collect::<HashSet<_>>();
    let mut used_ids = HashSet::with_capacity(records.len());
    for (index, record) in records.iter_mut().enumerate() {
        let assigned_id = reserve_unique_loaded_record_id(
            id(record),
            record_kind,
            index,
            &reserved_ids,
            &mut used_ids,
        );
        set_id(record, assigned_id);
    }
}

fn reserve_unique_loaded_record_id(
    original_id: &str,
    record_kind: &str,
    record_position: usize,
    reserved_ids: &HashSet<String>,
    used_ids: &mut HashSet<String>,
) -> String {
    if !original_id.is_empty() && used_ids.insert(original_id.to_string()) {
        return original_id.to_string();
    }

    let base = if original_id.is_empty() {
        record_kind.to_string()
    } else {
        original_id.to_string()
    };
    let mut suffix = record_position.saturating_add(1);
    loop {
        let candidate = format!("{base}:loaded:{suffix}");
        if !reserved_ids.contains(&candidate) && used_ids.insert(candidate.clone()) {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
}
