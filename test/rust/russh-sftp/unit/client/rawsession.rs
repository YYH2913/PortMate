use bytes::Bytes;
use tokio::{
    io::{duplex, AsyncWriteExt, DuplexStream},
    sync::oneshot,
};

use super::*;
use crate::{protocol::Version, utils::read_packet};

async fn read_client_packet(stream: &mut DuplexStream) -> Packet {
    let mut encoded = read_packet(stream, u32::MAX).await.unwrap();
    Packet::try_from(&mut encoded).unwrap()
}

async fn write_server_packet(stream: &mut DuplexStream, packet: Packet) {
    let encoded = Bytes::try_from(packet).unwrap();
    stream.write_all(&encoded).await.unwrap();
}

async fn initialize_server(stream: &mut DuplexStream) {
    assert!(matches!(read_client_packet(stream).await, Packet::Init(_)));
    write_server_packet(stream, Version::default().into()).await;
}

fn long_timeout_config() -> Config {
    Config {
        request_timeout_secs: 30,
        ..Config::default()
    }
}

#[tokio::test]
async fn malformed_response_fails_pending_request_without_waiting_for_timeout() {
    let (client, mut server) = duplex(4096);
    let session = RawSftpSession::new_with_config(client, long_timeout_config());
    let (release_tx, release_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        initialize_server(&mut server).await;
        assert!(matches!(
            read_client_packet(&mut server).await,
            Packet::Lstat(_)
        ));
        server.write_all(&[0, 0, 0, 1, 0xff]).await.unwrap();
        let _ = release_rx.await;
    });

    session.init().await.unwrap();
    let error = time::timeout(Duration::from_secs(1), session.lstat("/malformed"))
        .await
        .expect("malformed response waited for the request timeout")
        .unwrap_err();
    assert!(error.to_string().contains("unknown type"), "{error}");

    let _ = release_tx.send(());
    server_task.await.unwrap();
}

#[tokio::test]
async fn zero_length_response_fails_pending_request_without_waiting_for_timeout() {
    let (client, mut server) = duplex(4096);
    let session = RawSftpSession::new_with_config(client, long_timeout_config());
    let (release_tx, release_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        initialize_server(&mut server).await;
        assert!(matches!(
            read_client_packet(&mut server).await,
            Packet::Lstat(_)
        ));
        server.write_all(&0_u32.to_be_bytes()).await.unwrap();
        let _ = release_rx.await;
    });

    session.init().await.unwrap();
    let error = time::timeout(Duration::from_secs(1), session.lstat("/zero-length"))
        .await
        .expect("zero-length response waited for the request timeout")
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("only 0 bytes remaining, but 1 requested"),
        "{error}"
    );

    let _ = release_tx.send(());
    server_task.await.unwrap();
}

#[tokio::test]
async fn malformed_status_payload_fails_pending_request_without_waiting_for_timeout() {
    let (client, mut server) = duplex(4096);
    let session = RawSftpSession::new_with_config(client, long_timeout_config());
    let (release_tx, release_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        initialize_server(&mut server).await;
        let request_id = read_client_packet(&mut server).await.get_request_id();
        server.write_all(&5_u32.to_be_bytes()).await.unwrap();
        server.write_all(&[101]).await.unwrap();
        server.write_all(&request_id.to_be_bytes()).await.unwrap();
        let _ = release_rx.await;
    });

    session.init().await.unwrap();
    let error = time::timeout(Duration::from_secs(1), session.lstat("/malformed-status"))
        .await
        .expect("malformed status response waited for the request timeout")
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("only 0 bytes remaining, but 4 requested"),
        "{error}"
    );

    let _ = release_tx.send(());
    server_task.await.unwrap();
}

#[tokio::test]
async fn truncated_response_fails_pending_request_without_waiting_for_timeout() {
    let (client, mut server) = duplex(4096);
    let session = RawSftpSession::new_with_config(client, long_timeout_config());
    let server_task = tokio::spawn(async move {
        initialize_server(&mut server).await;
        assert!(matches!(
            read_client_packet(&mut server).await,
            Packet::Lstat(_)
        ));
        server.write_all(&[0, 0, 0, 9, 101]).await.unwrap();
    });

    session.init().await.unwrap();
    let error = time::timeout(Duration::from_secs(1), session.lstat("/truncated"))
        .await
        .expect("truncated response waited for the request timeout")
        .unwrap_err();
    assert!(
        error.to_string().contains("Unexpected EOF on stream"),
        "{error}"
    );

    server_task.await.unwrap();
}

#[tokio::test]
async fn oversized_response_fails_before_reading_the_declared_payload() {
    let (client, mut server) = duplex(4096);
    let session = RawSftpSession::new_with_config(
        client,
        Config {
            max_packet_len: 64,
            ..long_timeout_config()
        },
    );
    let (release_tx, release_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        initialize_server(&mut server).await;
        assert!(matches!(
            read_client_packet(&mut server).await,
            Packet::Lstat(_)
        ));
        server.write_all(&65_u32.to_be_bytes()).await.unwrap();
        let _ = release_rx.await;
    });

    session.init().await.unwrap();
    let error = time::timeout(Duration::from_secs(1), session.lstat("/oversized"))
        .await
        .expect("oversized response waited for its declared payload")
        .unwrap_err();
    assert!(
        error.to_string().contains("packet length limit exceeded"),
        "{error}"
    );

    let _ = release_tx.send(());
    server_task.await.unwrap();
}

#[tokio::test]
async fn unknown_response_id_fails_pending_request_without_waiting_for_timeout() {
    let (client, mut server) = duplex(4096);
    let session = RawSftpSession::new_with_config(client, long_timeout_config());
    let (release_tx, release_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        initialize_server(&mut server).await;
        let request_id = read_client_packet(&mut server).await.get_request_id();
        write_server_packet(
            &mut server,
            Packet::status(request_id + 1, StatusCode::Failure, "wrong request id", ""),
        )
        .await;
        let _ = release_rx.await;
    });

    session.init().await.unwrap();
    let error = time::timeout(Duration::from_secs(1), session.lstat("/wrong-id"))
        .await
        .expect("unknown response id waited for the request timeout")
        .unwrap_err();
    assert!(error.to_string().contains("unknown recipient"), "{error}");

    let _ = release_tx.send(());
    server_task.await.unwrap();
}

#[tokio::test]
async fn reverse_order_responses_are_routed_to_their_request_ids() {
    let (client, mut server) = duplex(4096);
    let session = RawSftpSession::new_with_config(client, long_timeout_config());
    let (release_tx, release_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        initialize_server(&mut server).await;
        let first_id = read_client_packet(&mut server).await.get_request_id();
        let second_id = read_client_packet(&mut server).await.get_request_id();
        write_server_packet(&mut server, Packet::error(second_id, StatusCode::Ok)).await;
        write_server_packet(&mut server, Packet::error(first_id, StatusCode::Ok)).await;
        let _ = release_rx.await;
    });

    session.init().await.unwrap();
    let first = session
        .write_nowait("handle".to_owned(), 0, vec![1])
        .unwrap();
    let second = session
        .write_nowait("handle".to_owned(), 1, vec![2])
        .unwrap();
    let second_packet = time::timeout(Duration::from_secs(1), second)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let first_packet = time::timeout(Duration::from_secs(1), first)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(first_packet, Packet::Status(status) if status.id == 1));
    assert!(matches!(second_packet, Packet::Status(status) if status.id == 2));

    let _ = release_tx.send(());
    server_task.await.unwrap();
}
