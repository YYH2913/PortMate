use super::*;
use bytes::Bytes;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

use crate::{
    client::Config,
    protocol::{Data, Packet, Version},
    utils::read_packet,
};

async fn receive_packet(stream: &mut tokio::io::DuplexStream) -> Packet {
    let mut bytes = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        read_packet(stream, u32::MAX),
    )
    .await
    .expect("SFTP request not sent concurrently")
    .unwrap();
    Packet::try_from(&mut bytes).unwrap()
}

async fn send_packet(stream: &mut tokio::io::DuplexStream, packet: Packet) {
    stream
        .write_all(&Bytes::try_from(packet).unwrap())
        .await
        .unwrap();
}

async fn initialize(stream: &mut tokio::io::DuplexStream) {
    assert!(matches!(receive_packet(stream).await, Packet::Init(_)));
    send_packet(stream, Version::default().into()).await;
}

fn test_file(session: Arc<RawSftpSession>, read_limit: Option<u64>) -> File {
    File::new(
        session,
        "handle".to_owned(),
        Features {
            hardlink: false,
            fsync: false,
            statvfs: false,
            limits: read_limit.map(|read_len| crate::client::rawsession::Limits {
                read_len: Some(read_len),
                ..Default::default()
            }),
            max_concurrent_writes: 8,
            max_packet_len: 256 * 1024,
        },
    )
}

#[tokio::test]
async fn large_advertised_limits_are_clamped_before_integer_conversion() {
    let (client, mut server) = duplex(4096);
    let server_task = tokio::spawn(async move {
        assert!(matches!(receive_packet(&mut server).await, Packet::Init(_)));
        send_packet(
            &mut server,
            Version {
                extensions: [(crate::extensions::LIMITS.to_string(), "1".to_string())]
                    .into_iter()
                    .collect(),
                ..Default::default()
            }
            .into(),
        )
        .await;
        let Packet::Extended(request) = receive_packet(&mut server).await else {
            panic!("expected limits request");
        };
        assert_eq!(request.request, crate::extensions::LIMITS);
        let data = [1_u64 << 32, 1_u64 << 32, 1_u64 << 32, 32]
            .into_iter()
            .flat_map(u64::to_be_bytes)
            .collect();
        send_packet(
            &mut server,
            crate::protocol::ExtendedReply {
                id: request.id,
                data,
            }
            .into(),
        )
        .await;
        let Packet::Open(open) = receive_packet(&mut server).await else {
            panic!("expected OPEN");
        };
        send_packet(
            &mut server,
            crate::protocol::Handle {
                id: open.id,
                handle: "handle".to_string(),
            }
            .into(),
        )
        .await;
        let Packet::Read(read) = receive_packet(&mut server).await else {
            panic!("expected READ");
        };
        assert_eq!(read.len, 4);
        send_packet(
            &mut server,
            Data {
                id: read.id,
                data: vec![1, 2, 3, 4],
            }
            .into(),
        )
        .await;
        let Packet::Close(close) = receive_packet(&mut server).await else {
            panic!("expected CLOSE");
        };
        send_packet(&mut server, Packet::error(close.id, StatusCode::Ok)).await;
    });
    let session = crate::client::SftpSession::new(client).await.unwrap();
    let mut file = session.open("file").await.unwrap();
    assert_eq!(file.preferred_write_packet_size(), 256 * 1024 - 31);
    let mut buffer = [0_u8; 4];
    assert_eq!(file.read_pipelined(&mut buffer, 8).await.unwrap(), 4);
    assert_eq!(buffer, [1, 2, 3, 4]);
    file.shutdown().await.unwrap();
    server_task.await.unwrap();
}

#[tokio::test]
async fn aligned_write_buffers_reduce_tiny_tail_requests_without_changing_data() {
    for aligned in [false, true] {
        let (client, mut server) = duplex(4096);
        let session = Arc::new(RawSftpSession::new(client));
        let server_task = tokio::spawn(async move {
            initialize(&mut server).await;
            let mut uploaded = Vec::new();
            let mut count = 0;
            loop {
                match receive_packet(&mut server).await {
                    Packet::Write(write) => {
                        assert_eq!(write.offset, uploaded.len() as u64);
                        assert!(write.data.len() <= 240);
                        uploaded.extend_from_slice(&write.data);
                        count += 1;
                        send_packet(&mut server, Packet::error(write.id, StatusCode::Ok)).await;
                    }
                    Packet::Close(close) => {
                        send_packet(&mut server, Packet::error(close.id, StatusCode::Ok)).await;
                        break;
                    }
                    packet => panic!("unexpected packet: {packet:?}"),
                }
            }
            (uploaded, count)
        });
        session.init().await.unwrap();
        let mut file = test_file(session, None);
        file.features.limits = Some(crate::client::rawsession::Limits {
            write_len: Some(240),
            ..Default::default()
        });
        let payload: Vec<_> = (0..512).map(|index| index as u8).collect();
        let chunk_size = if aligned {
            file.preferred_write_packet_size()
        } else {
            256
        };
        for chunk in payload.chunks(chunk_size) {
            file.write_all(chunk).await.unwrap();
        }
        file.flush().await.unwrap();
        file.shutdown().await.unwrap();
        let (uploaded, requests) = server_task.await.unwrap();
        assert_eq!(uploaded, payload);
        assert_eq!(requests, if aligned { 3 } else { 4 });
    }
}

#[tokio::test]
async fn pipelined_write_acks_fail_promptly_on_timeout_and_stream_disconnect() {
    for timeout in [false, true] {
        let (client, mut server) = duplex(4096);
        let session = Arc::new(RawSftpSession::new(client));
        let (release_tx, release_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            initialize(&mut server).await;
            assert!(matches!(
                receive_packet(&mut server).await,
                Packet::Write(_)
            ));
            if timeout {
                let _ = release_rx.await;
            }
            // No STATUS is returned: either disconnect, or withhold the ACK.
        });
        session.init().await.unwrap();
        if timeout {
            session.set_timeout(0);
        }
        let mut file = test_file(session, None);
        file.write_all(&[1, 2, 3]).await.unwrap();
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), file.flush())
            .await
            .expect("write ACK wait ignored timeout/transport failure")
            .unwrap_err();
        if timeout {
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        }
        let _ = release_tx.send(());
        server_task.await.unwrap();
    }
}

#[tokio::test]
async fn preferred_write_buffers_respect_limits_framing_and_legacy_fallback() {
    let (client, _server) = duplex(4096);
    let session = Arc::new(RawSftpSession::new(client));
    let mut file = test_file(session, None);
    assert_eq!(file.preferred_write_packet_size(), 64 * 1024);
    file.features.limits = Some(crate::client::rawsession::Limits {
        write_len: Some(128 * 1024),
        ..Default::default()
    });
    assert_eq!(file.preferred_write_packet_size(), 128 * 1024);
    file.features.max_packet_len = 100;
    assert_eq!(file.preferred_write_packet_size(), 69); // Framing + header + handle: 4 + 21 + 6 bytes.
    file.features.limits = Some(crate::client::rawsession::Limits {
        packet_len: Some(80),
        write_len: Some(u64::MAX),
        ..Default::default()
    });
    assert_eq!(file.preferred_write_packet_size(), 49);
    file.features.limits.as_mut().unwrap().write_len = Some(8);
    assert_eq!(file.preferred_write_packet_size(), 8);
    file.features.max_packet_len = 9;
    assert_eq!(file.preferred_write_packet_size(), 0);
    file.closed = true;
}

#[tokio::test]
async fn pipelined_reads_fill_short_replies_in_order_and_preserve_seek_and_eof() {
    let (client, mut server) = duplex(256 * 1024);
    let session = Arc::new(RawSftpSession::new(client));
    let server_task = tokio::spawn(async move {
        initialize(&mut server).await;
        let mut requests = Vec::new();
        for _ in 0..3 {
            let Packet::Read(read) = receive_packet(&mut server).await else {
                panic!("expected READ");
            };
            assert_eq!(read.len, 8);
            requests.push(read);
        }
        assert_eq!(
            requests.iter().map(|r| r.offset).collect::<Vec<_>>(),
            [0, 8, 16]
        );
        // No reply was sent until all three requests arrived. Respond in reverse
        // order with short DATA and require gap-filling requests for each range.
        for read in requests.into_iter().rev() {
            let end = (read.offset as usize + 4).min(19);
            send_packet(
                &mut server,
                Data {
                    id: read.id,
                    data: (read.offset as u8..end as u8).collect(),
                }
                .into(),
            )
            .await;
        }
        loop {
            match receive_packet(&mut server).await {
                Packet::Read(read) if read.offset >= 19 => {
                    send_packet(&mut server, Packet::error(read.id, StatusCode::Eof)).await;
                }
                Packet::Read(read) => {
                    assert!(read.len <= 8);
                    let end = (read.offset as usize + (read.len as usize).min(4)).min(19);
                    send_packet(
                        &mut server,
                        Data {
                            id: read.id,
                            data: (read.offset as u8..end as u8).collect(),
                        }
                        .into(),
                    )
                    .await;
                }
                Packet::Close(close) => {
                    send_packet(&mut server, Packet::error(close.id, StatusCode::Ok)).await;
                    break;
                }
                packet => panic!("unexpected packet: {packet:?}"),
            }
        }
    });
    session.init().await.unwrap();
    let mut file = test_file(session, Some(8));
    let mut buffer = [0_u8; 24];
    assert_eq!(file.read_pipelined(&mut buffer, 3).await.unwrap(), 19);
    assert_eq!(&buffer[..19], &(0..19_u8).collect::<Vec<_>>());
    assert_eq!(file.pos, 19);
    tokio::io::AsyncSeekExt::seek(&mut file, SeekFrom::Start(3))
        .await
        .unwrap();
    assert_eq!(file.read_pipelined(&mut buffer, 3).await.unwrap(), 16);
    assert_eq!(&buffer[..16], &(3..19_u8).collect::<Vec<_>>());
    assert_eq!(file.read_pipelined(&mut buffer, 3).await.unwrap(), 0);
    file.shutdown().await.unwrap();
    server_task.await.unwrap();
}

#[tokio::test]
async fn pipelined_reads_bound_window_and_drain_on_cancel_before_close() {
    let (client, mut server) = duplex(1024 * 1024);
    let session = Arc::new(RawSftpSession::new(client));
    let (sent_tx, sent_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        initialize(&mut server).await;
        let mut reads = Vec::new();
        for _ in 0..16 {
            let Packet::Read(read) = receive_packet(&mut server).await else {
                panic!("expected READ");
            };
            assert_eq!(read.len, PIPELINED_READ_PAYLOAD);
            reads.push(read);
        }
        sent_tx.send(()).unwrap();
        // Dropping the caller and then the file must not send CLOSE ahead of
        // the pending reads, or create a seventeenth speculative request.
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(30),
            read_packet(&mut server, u32::MAX)
        )
        .await
        .is_err());
        for read in reads.into_iter().rev() {
            send_packet(
                &mut server,
                Data {
                    id: read.id,
                    data: vec![0x5a; read.len as usize],
                }
                .into(),
            )
            .await;
        }
        let Packet::Close(close) = receive_packet(&mut server).await else {
            panic!("expected CLOSE");
        };
        send_packet(&mut server, Packet::error(close.id, StatusCode::Ok)).await;
    });
    session.init().await.unwrap();
    let mut file = test_file(session, None);
    let mut buffer = vec![0_u8; 1024 * 1024];
    {
        let read = file.read_pipelined(&mut buffer, usize::MAX);
        tokio::pin!(read);
        tokio::select! {
            result = &mut read => panic!("read completed before cancellation: {result:?}"),
            result = sent_rx => result.unwrap(),
        }
    }
    assert!(tokio::io::AsyncSeekExt::seek(&mut file, SeekFrom::Start(0))
        .await
        .is_err());
    assert_eq!(
        file.write_all(&[1]).await.unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    drop(file);
    server_task.await.unwrap();
}

#[tokio::test]
async fn pipelined_reads_reject_empty_oversized_and_inconsistent_eof_data() {
    for case in ["empty", "oversized", "data-after-eof", "permission"] {
        let (client, mut server) = duplex(4096);
        let session = Arc::new(RawSftpSession::new(client));
        let server_task = tokio::spawn(async move {
            initialize(&mut server).await;
            let mut reads = Vec::new();
            for _ in 0..2 {
                let Packet::Read(read) = receive_packet(&mut server).await else {
                    panic!("expected READ");
                };
                reads.push(read);
            }
            for read in reads {
                let packet = match (case, read.offset) {
                    ("data-after-eof", 0) => Packet::error(read.id, StatusCode::Eof),
                    ("permission", 0) => Packet::error(read.id, StatusCode::PermissionDenied),
                    ("empty", 0) => Data {
                        id: read.id,
                        data: vec![],
                    }
                    .into(),
                    ("oversized", 0) => Data {
                        id: read.id,
                        data: vec![0; read.len as usize + 1],
                    }
                    .into(),
                    _ => Data {
                        id: read.id,
                        data: vec![0x5a; read.len as usize],
                    }
                    .into(),
                };
                send_packet(&mut server, packet).await;
            }
            let Packet::Close(close) = receive_packet(&mut server).await else {
                panic!("expected CLOSE");
            };
            send_packet(&mut server, Packet::error(close.id, StatusCode::Ok)).await;
        });
        session.init().await.unwrap();
        let mut file = test_file(session, Some(8));
        let mut buffer = [0_u8; 16];
        let error = file.read_pipelined(&mut buffer, 2).await.unwrap_err();
        if case != "permission" {
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
        assert_eq!(file.pos, 0);
        assert_eq!(buffer, [0; 16]);
        file.shutdown().await.unwrap();
        server_task.await.unwrap();
    }
}

#[tokio::test]
async fn pipelined_reads_use_one_batch_deadline_and_handle_empty_buffers() {
    let (client, mut server) = duplex(4096);
    let session = Arc::new(RawSftpSession::new(client));
    let server_task = tokio::spawn(async move {
        initialize(&mut server).await;
        // Withhold responses and wait for the batch to close its SFTP stream.
        let mut data = Vec::new();
        server.read_to_end(&mut data).await.unwrap();
    });
    session.init().await.unwrap();
    session.set_timeout(0);
    let mut file = test_file(session, None);
    assert_eq!(file.read_pipelined(&mut [], 8).await.unwrap(), 0);
    let error = file.read_pipelined(&mut [0_u8; 8], 8).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(file.pos, 0);
    drop(file);
    server_task.await.unwrap();
}

#[tokio::test]
async fn pipelined_reads_respect_packet_limits_zero_windows_and_offset_bounds() {
    let (client, mut server) = duplex(4096);
    let session = Arc::new(RawSftpSession::new(client));
    let server_task = tokio::spawn(async move {
        initialize(&mut server).await;
        let Packet::Read(read) = receive_packet(&mut server).await else {
            panic!("expected READ");
        };
        assert_eq!(read.len, 23); // max_packet_len (32) minus DATA overhead (9).
        send_packet(
            &mut server,
            Data {
                id: read.id,
                data: vec![0x5a; 23],
            }
            .into(),
        )
        .await;
        let Packet::Close(close) = receive_packet(&mut server).await else {
            panic!("expected CLOSE");
        };
        send_packet(&mut server, Packet::error(close.id, StatusCode::Ok)).await;
    });
    session.init().await.unwrap();
    let mut file = test_file(session, None);
    file.features.max_packet_len = 32;
    let mut buffer = [0_u8; 64];
    assert_eq!(file.read_pipelined(&mut buffer, 0).await.unwrap(), 23);
    assert_eq!(file.pos, 23);
    file.pos = u64::MAX;
    assert_eq!(
        file.read_pipelined(&mut buffer, 8)
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    file.features.max_packet_len = 9;
    file.pos = 0;
    assert_eq!(
        file.read_pipelined(&mut buffer, 8)
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    file.shutdown().await.unwrap();
    server_task.await.unwrap();
}

#[tokio::test]
#[ignore = "opt-in SFTP throughput benchmark with injected response latency"]
async fn read_pipeline_latency_benchmark() {
    for window in [1, 4, 8, 16] {
        let (client, server) = duplex(1024 * 1024);
        let session = Arc::new(RawSftpSession::new(client));
        let payload: Arc<Vec<u8>> =
            Arc::new((0..8 * 1024 * 1024).map(|index| index as u8).collect());
        let server_payload = Arc::clone(&payload);
        let server_task = tokio::spawn(async move {
            let (mut reader, mut writer) = tokio::io::split(server);
            let (reply_tx, mut reply_rx) = tokio::sync::mpsc::channel::<Packet>(32);
            let writer_task = tokio::spawn(async move {
                while let Some(packet) = reply_rx.recv().await {
                    writer
                        .write_all(&Bytes::try_from(packet).unwrap())
                        .await
                        .unwrap();
                }
            });
            let mut replies = tokio::task::JoinSet::new();
            loop {
                while replies.try_join_next().is_some() {}
                let mut encoded = read_packet(&mut reader, u32::MAX).await.unwrap();
                match Packet::try_from(&mut encoded).unwrap() {
                    Packet::Init(_) => reply_tx.send(Version::default().into()).await.unwrap(),
                    Packet::Read(read) => {
                        let tx = reply_tx.clone();
                        let payload = Arc::clone(&server_payload);
                        replies.spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                            let offset = read.offset as usize;
                            let packet = if offset >= payload.len() {
                                Packet::error(read.id, StatusCode::Eof)
                            } else {
                                Data {
                                    id: read.id,
                                    data: payload
                                        [offset..(offset + read.len as usize).min(payload.len())]
                                        .to_vec(),
                                }
                                .into()
                            };
                            tx.send(packet).await.unwrap();
                        });
                    }
                    Packet::Close(close) => {
                        while replies.join_next().await.is_some() {}
                        reply_tx
                            .send(Packet::error(close.id, StatusCode::Ok))
                            .await
                            .unwrap();
                        break;
                    }
                    packet => panic!("unexpected packet: {packet:?}"),
                }
            }
            drop(reply_tx);
            writer_task.await.unwrap();
        });
        session.init().await.unwrap();
        let mut file = test_file(session, None);
        let mut buffer = vec![0_u8; 32 * 1024 * window];
        let mut downloaded = Vec::with_capacity(payload.len());
        let started = std::time::Instant::now();
        loop {
            let read = file.read_pipelined(&mut buffer, window).await.unwrap();
            if read == 0 {
                break;
            }
            downloaded.extend_from_slice(&buffer[..read]);
        }
        let elapsed = started.elapsed().as_secs_f64();
        println!("SFTP_LATENCY window={window} response_delay_ms=2 seconds={elapsed:.6} mib_per_second={:.3}",8.0/elapsed);
        assert_eq!(&downloaded, &*payload);
        file.shutdown().await.unwrap();
        server_task.await.unwrap();
    }
}

#[tokio::test]
async fn oversized_read_response_returns_invalid_data_without_panicking() {
    let (client, mut server) = duplex(4096);
    let session = Arc::new(RawSftpSession::new_with_config(
        client,
        Config {
            request_timeout_secs: 5,
            ..Config::default()
        },
    ));
    let server_task = tokio::spawn(async move {
        let mut init = read_packet(&mut server, u32::MAX).await.unwrap();
        assert!(matches!(
            Packet::try_from(&mut init).unwrap(),
            Packet::Init(_)
        ));
        server
            .write_all(&Bytes::try_from(Packet::Version(Version::default())).unwrap())
            .await
            .unwrap();

        let mut read = read_packet(&mut server, u32::MAX).await.unwrap();
        let id = match Packet::try_from(&mut read).unwrap() {
            Packet::Read(read) => read.id,
            packet => panic!("expected READ packet, got {packet:?}"),
        };
        server
            .write_all(
                &Bytes::try_from(Packet::Data(Data {
                    id,
                    data: vec![b'x'; 5],
                }))
                .unwrap(),
            )
            .await
            .unwrap();
    });

    session.init().await.unwrap();
    let mut file = File::new(
        session,
        "handle".to_string(),
        Features {
            hardlink: false,
            fsync: false,
            statvfs: false,
            limits: None,
            max_concurrent_writes: 1,
            max_packet_len: 1024,
        },
    );
    let mut buffer = [0_u8; 4];
    let error = file.read(&mut buffer).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("more data than requested"));
    server_task.await.unwrap();
}
