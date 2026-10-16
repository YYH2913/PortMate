use super::*;

struct TftpCadenceSample {
    data_elapsed: Duration,
    ack_to_data: Vec<Duration>,
    task_elapsed: Duration,
}

/// Exercise the actual transfer runner, including U-Boot command setup, UDP
/// DATA/ACK, source reads, connection checks, progress and optional throttling.
async fn measure_tftp_cadence(
    block_size: usize,
    negotiated: bool,
    ack_delay: Duration,
    full_blocks: usize,
    rate_limit: Option<u64>,
    block_store_checkpoint: bool,
) -> TftpCadenceSample {
    let root = canonical_test_tempdir();
    let store_path = root.path().join("store.sqlite3");
    let peer_store_path = store_path.clone();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let payload = (0..block_size * full_blocks + 11)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let expected = payload.clone();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let (progress_tx, progress_rx) =
        tokio::sync::oneshot::channel::<(Arc<Mutex<SessionStore>>, String)>();
    let device = tokio::spawn(async move {
        let (mut terminal, _) = listener.accept().await.unwrap();
        let mut commands = Vec::new();
        let mut command_times = Vec::new();
        while command_times.len() < 4 {
            let byte = terminal.read_u8().await.unwrap();
            commands.push(byte);
            if byte == b'\r' {
                command_times.push(Instant::now());
            }
        }
        let command_text = String::from_utf8(commands).unwrap();
        let (progress_store, progress_task_id) = progress_rx.await.unwrap();
        let port = command_text
            .split('\r')
            .find_map(|line| line.strip_prefix("setenv tftpdstp "))
            .unwrap()
            .parse::<u16>()
            .unwrap();
        let client = tokio::net::UdpSocket::bind(("127.0.0.1", 0)).await.unwrap();
        let mut request = b"\0\x01cadence.bin\0octet\0".to_vec();
        if negotiated {
            request.extend_from_slice(format!("blksize\0{block_size}\0tsize\x000\0").as_bytes());
        }
        client.send_to(&request, ("127.0.0.1", port)).await.unwrap();
        let mut packet = vec![0u8; 2048];
        if negotiated {
            let (size, peer) = client.recv_from(&mut packet).await.unwrap();
            assert_eq!(&packet[..2], &[0, 6]);
            assert!(packet[..size]
                .windows(format!("blksize\0{block_size}\0").len())
                .any(|value| value == format!("blksize\0{block_size}\0").as_bytes()));
            client.send_to(&[0, 4, 0, 0], peer).await.unwrap();
        }
        let mut received = Vec::new();
        let mut expected_block = 1u16;
        let mut first_data = None;
        let mut previous_ack = None;
        let mut ack_to_data = Vec::new();
        let mut checkpoint_guard = None;
        let mut live_progress = HashSet::new();
        let data_elapsed = loop {
            let (size, peer) =
                tokio::time::timeout(Duration::from_secs(5), client.recv_from(&mut packet))
                    .await
                    .expect("TFTP DATA did not follow its ACK")
                    .unwrap();
            let received_at = Instant::now();
            if first_data.is_none() && block_store_checkpoint {
                // Lifecycle writes have already committed. Hold the real
                // Store snapshot lock until all DATA has arrived: an old
                // per-block progress checkpoint would stall on this lock.
                checkpoint_guard = Some(lock_store_snapshot(&peer_store_path).unwrap());
            }
            let started = *first_data.get_or_insert(received_at);
            if let Some(ack) = previous_ack {
                ack_to_data.push(received_at.saturating_duration_since(ack));
            }
            assert_eq!(&packet[..2], &[0, 3]);
            assert_eq!(u16::from_be_bytes([packet[2], packet[3]]), expected_block);
            if block_store_checkpoint {
                // Permit brief connection/progress mutex contention, but not
                // an I/O wait while holding the shared Store.
                let task = tokio::time::timeout(Duration::from_secs(1), async {
                    loop {
                        let next = progress_store
                            .try_lock()
                            .ok()
                            .and_then(|store| store.transfer_by_id(&progress_task_id));
                        if let Some(task) = next {
                            break task;
                        }
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
                .expect("progress must not hold the Store while waiting for disk");
                assert_eq!(task.status, TransferStatus::Running);
                live_progress.insert(task.bytes_done);
                if expected_block > 1 {
                    assert!(
                        task.bytes_done > 0,
                        "live byte counts must update before completion"
                    );
                }
            }
            received.extend_from_slice(&packet[4..size]);
            if !ack_delay.is_zero() {
                tokio::time::sleep(ack_delay).await;
            }
            client
                .send_to(&[0, 4, packet[2], packet[3]], peer)
                .await
                .unwrap();
            previous_ack = Some(Instant::now());
            if size < 4 + block_size {
                break received_at.saturating_duration_since(started);
            }
            expected_block = expected_block.wrapping_add(1);
        };
        // Completion must still be persisted after the data phase.
        drop(checkpoint_guard);
        assert_eq!(received, expected);
        if block_store_checkpoint {
            live_progress.remove(&0);
            assert!(
                live_progress.len() >= 2,
                "blocked persistence must not suppress ongoing progress updates"
            );
        }
        let command_gaps_ms = command_times
            .windows(2)
            .map(|times| times[1].duration_since(times[0]).as_secs_f64() * 1000.0)
            .collect::<Vec<_>>();
        let _ = release_rx.await;
        (
            data_elapsed,
            ack_to_data,
            command_gaps_ms,
            live_progress.len(),
        )
    });

    let mut profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
        host: "127.0.0.1".into(),
        port: address.port(),
        reconnect: false,
        ..Default::default()
    }));
    profile.transfer.rate_limit_bytes_per_second = rate_limit;
    let source = root.path().join("cadence.bin");
    fs::write(&source, &payload).unwrap();
    let state = test_app_state(profile.clone(), store_path);
    open_tcp_session(&state, profile.clone()).await.unwrap();
    let started = Instant::now();
    let task = start_transfer_inner(
        &state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Tftp,
            source: source.display().to_string(),
            destination:
                "load:tftpboot?deviceIp=127.0.0.1&serverIp=127.0.0.1&bindPort=0&timeoutSeconds=10"
                    .into(),
        },
    )
    .await
    .unwrap();
    assert!(progress_tx
        .send((Arc::clone(&state.store), task.id.clone()))
        .is_ok());
    let completed = wait_for_transfer_terminal_state(&state, &task.id).await;
    let task_elapsed = started.elapsed();
    let _ = release_tx.send(());
    let (data_elapsed, ack_to_data, command_gaps_ms, live_progress_snapshots) =
        tokio::time::timeout(Duration::from_secs(2), device)
            .await
            .expect("TFTP cadence device did not finish")
            .unwrap();
    close_session_inner(&state, profile.id.clone())
        .await
        .unwrap();
    assert_eq!(completed.status, TransferStatus::Completed, "{completed:?}");
    assert_eq!(completed.bytes_done, payload.len() as u64);
    let persisted = load_store_sqlite(&state.store_path).unwrap();
    let persisted_task = persisted.transfer_by_id(&task.id).unwrap();
    assert_eq!(persisted_task.status, TransferStatus::Completed);
    assert_eq!(persisted_task.bytes_done, payload.len() as u64);
    assert_eq!(ack_to_data.len(), full_blocks);
    let mut ack_gaps_ms = ack_to_data
        .iter()
        .map(|gap| gap.as_secs_f64() * 1000.0)
        .collect::<Vec<_>>();
    ack_gaps_ms.sort_by(f64::total_cmp);
    eprintln!(
        "TFTP_CADENCE {}",
        serde_json::json!({
            "blockBytes": block_size,
            "blocks": full_blocks + 1,
            "bytes": payload.len(),
            "negotiated": negotiated,
            "peerAckDelayMs": ack_delay.as_millis(),
            "rateLimitBytesPerSecond": rate_limit,
            "storeCheckpointBlocked": block_store_checkpoint,
            "liveProgressSnapshots": live_progress_snapshots,
            "commandGapsMs": command_gaps_ms,
            "dataElapsedMs": data_elapsed.as_secs_f64() * 1000.0,
            "taskElapsedMs": task_elapsed.as_secs_f64() * 1000.0,
            "ackToDataMedianMs": ack_gaps_ms[ack_gaps_ms.len() / 2],
            "ackToDataMaxMs": ack_gaps_ms.last(),
        })
    );
    TftpCadenceSample {
        data_elapsed,
        ack_to_data,
        task_elapsed,
    }
}

#[test]
fn tftp_data_blocks_follow_ack_without_fixed_console_pacing() {
    tauri::async_runtime::block_on(async {
        for (size, negotiated, delay) in [
            (512, false, Duration::ZERO),
            (1468, true, Duration::ZERO),
            (512, false, Duration::from_millis(20)),
        ] {
            let sample = measure_tftp_cadence(size, negotiated, delay, 64, None, false).await;
            // Generous enough for loaded CI; a 500 ms pause per block takes
            // at least 32 seconds and cannot pass this data-phase bound.
            assert!(sample.data_elapsed < Duration::from_secs(6));
            assert_eq!(sample.ack_to_data.len(), 64);
        }
    });
}

#[test]
fn tftp_explicit_rate_limit_can_intentionally_create_half_second_blocks() {
    tauri::async_runtime::block_on(async {
        let sample = measure_tftp_cadence(512, false, Duration::ZERO, 8, Some(1024), false).await;
        // Startup time counts towards the existing shared rate limiter, so
        // early blocks can burst; later 512-byte blocks are paced at 1 KiB/s.
        assert!(sample.task_elapsed >= Duration::from_secs(4));
        assert!(sample
            .ack_to_data
            .iter()
            .any(|gap| *gap >= Duration::from_millis(400)));
    });
}

#[test]
fn tftp_data_progress_does_not_wait_for_store_checkpoints() {
    tauri::async_runtime::block_on(async {
        let sample =
            measure_tftp_cadence(1468, true, Duration::from_millis(20), 64, None, true).await;
        assert!(sample.data_elapsed < Duration::from_secs(6));
    });
}
