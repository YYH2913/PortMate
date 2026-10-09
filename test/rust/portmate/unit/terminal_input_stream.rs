use super::*;

#[test]
fn destroying_one_window_clears_only_its_input_streams() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("store.sqlite3");
    let other_path = root.path().join("other.sqlite3");
    let stream = |id: &str| InputStream {
        id: id.to_string(),
        runtime_id: format!("runtime-{id}"),
        ordered: OrderedInput::default(),
    };
    let streams = STREAMS.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let mut entries = streams.lock().unwrap();
        entries.insert(
            (path.clone(), "session".into(), "main".into()),
            stream("main"),
        );
        entries.insert(
            (path.clone(), "session".into(), "detached".into()),
            stream("detached"),
        );
        entries.insert(
            (other_path.clone(), "session".into(), "main".into()),
            stream("other"),
        );
    }
    clear_owner_streams(&path, "main");
    {
        let entries = streams.lock().unwrap();
        assert!(!entries.contains_key(&(path.clone(), "session".into(), "main".into())));
        assert!(entries.contains_key(&(path.clone(), "session".into(), "detached".into())));
        assert!(entries.contains_key(&(other_path.clone(), "session".into(), "main".into())));
    }
    clear_session_streams(&path, "session");
    clear_session_streams(&other_path, "session");
}

fn packet(text: &str, sensitive: bool) -> InputPacket {
    InputPacket {
        text: Zeroizing::new(text.into()),
        coalesce: !text.contains('\r'),
        sensitive,
    }
}

#[test]
fn restores_packet_order_without_waiting_for_ipc_responses() {
    let mut ordered = OrderedInput::default();
    let mut sent = Vec::new();
    for (sequence, text, sensitive) in [
        (1, "\x7f", true),
        (3, "\r", false),
        (0, "a", false),
        (2, "b", true),
    ] {
        ordered
            .accept(sequence, packet(text, sensitive), |packet| {
                sent.push((packet.text.to_string(), packet.sensitive));
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(
        sent,
        [
            ("a".into(), false),
            ("\x7f".into(), true),
            ("b".into(), true),
            ("\r".into(), false)
        ]
    );
    assert!(ordered.pending.is_empty());
    assert_eq!(ordered.pending_bytes, 0);
}

#[test]
fn retries_do_not_duplicate_pending_or_admitted_input() {
    let mut ordered = OrderedInput::default();
    let mut sent = String::new();
    for (sequence, text) in [(1, "b"), (1, "b"), (0, "a"), (0, "a"), (1, "b")] {
        ordered
            .accept(sequence, packet(text, false), |packet| {
                sent.push_str(&packet.text);
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(sent, "ab");
}

#[test]
fn admission_failure_cancels_buffered_suffix_instead_of_reordering_it() {
    let mut ordered = OrderedInput::default();
    ordered
        .accept(1, packet("later", true), |_| panic!("missing prefix"))
        .unwrap();
    assert!(ordered
        .accept(0, packet("first", false), |_| Err("queue full".into()))
        .is_err());
    assert!(ordered.pending.is_empty());
    assert!(ordered
        .accept(2, packet("never", false), |_| panic!(
            "failed stream resumed"
        ))
        .is_err());
}

#[test]
fn bounds_sequence_window_and_buffered_bytes() {
    assert!(OrderedInput::default()
        .accept(32, packet("x", false), |_| Ok(()))
        .is_err());
    assert!(OrderedInput::default()
        .accept(0, packet(&"x".repeat(MAX_PACKET_BYTES + 1), false), |_| Ok(
            ()
        ))
        .is_err());
    let mut ordered = OrderedInput::default();
    for sequence in 1..=8 {
        ordered
            .accept(
                sequence,
                packet(&"x".repeat(MAX_PACKET_BYTES), true),
                |_| panic!("missing prefix"),
            )
            .unwrap();
    }
    assert!(ordered.accept(9, packet("x", true), |_| Ok(())).is_err());
    assert!(ordered.pending.is_empty());
}
