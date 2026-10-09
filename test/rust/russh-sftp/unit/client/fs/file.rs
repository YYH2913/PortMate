use super::*;
use bytes::Bytes;
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

use crate::{
    client::Config,
    protocol::{Data, Packet, Version},
    utils::read_packet,
};

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
