use super::*;
use russh_sftp::protocol::{
    Attrs, FileAttr, FileAttributeExtension, FileAttributes, Status, Version,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

async fn read_permissions_request(stream: &mut DuplexStream) -> Vec<u8> {
    let length = stream.read_u32().await.unwrap();
    assert!(length <= 4096);
    let mut packet = vec![0; length as usize];
    stream.read_exact(&mut packet).await.unwrap();
    packet
}

async fn write_permissions_reply<T: serde::Serialize>(
    stream: &mut DuplexStream,
    packet_type: u8,
    value: &T,
) {
    let payload = russh_sftp::ser::to_bytes(value).unwrap();
    stream.write_u32(payload.len() as u32 + 1).await.unwrap();
    stream.write_u8(packet_type).await.unwrap();
    stream.write_all(&payload).await.unwrap();
}

#[test]
fn russh_chmod_emits_only_permissions_for_files_and_directories() {
    tauri::async_runtime::block_on(async {
        let cases = [("/active.log", 0o100640), ("/directory", 0o40750)];
        let (client, mut server) = tokio::io::duplex(4096);
        let server_task = tokio::spawn(async move {
            assert_eq!(read_permissions_request(&mut server).await[0], 1);
            write_permissions_reply(&mut server, 2, &Version::default()).await;
            let mut captured = Vec::new();
            for (path, permissions) in cases {
                let lstat = read_permissions_request(&mut server).await;
                assert_eq!(lstat[0], 7);
                assert_eq!(&lstat[9..], path.as_bytes());
                let id = u32::from_be_bytes(lstat[1..5].try_into().unwrap());
                let mut attributes = FileAttributes::empty();
                attributes.size = Some(11);
                attributes.uid = Some(1000);
                attributes.gid = Some(2000);
                attributes.permissions = Some((permissions & 0o170000) | 0o777);
                attributes.atime = Some(123);
                attributes.mtime = Some(456);
                attributes.extended = vec![FileAttributeExtension {
                    extended_type: "vendor@example.com".to_string(),
                    extended_data: vec![0, 0xff],
                }];
                write_permissions_reply(
                    &mut server,
                    105,
                    &Attrs {
                        id,
                        attrs: attributes,
                    },
                )
                .await;

                // Capture the actual backend's framed SETSTAT packet before
                // replying; no reconstructed chmod payload is used here.
                let setstat = read_permissions_request(&mut server).await;
                let id = u32::from_be_bytes(setstat[1..5].try_into().unwrap());
                captured.push((path, permissions, setstat));
                write_permissions_reply(
                    &mut server,
                    101,
                    &Status {
                        id,
                        status_code: russh_sftp::protocol::StatusCode::Ok,
                        error_message: String::new(),
                        language_tag: String::new(),
                    },
                )
                .await;
            }
            captured
        });

        let sftp = SftpBackendSession::from_russh(SftpSession::new(client).await.unwrap());
        for (path, permissions) in cases {
            let mut metadata = sftp.symlink_metadata(path.to_string()).await.unwrap();
            assert_eq!(metadata.len(), 11);
            metadata.permissions = Some(permissions);
            sftp.set_metadata(path.to_string(), metadata).await.unwrap();
        }
        if let SftpBackendSession::Russh(session) = &sftp {
            session.close().await.unwrap();
        }
        for (path, permissions, packet) in server_task.await.unwrap() {
            assert_eq!(packet[0], 9, "chmod must send SETSTAT");
            let path_length = u32::from_be_bytes(packet[5..9].try_into().unwrap()) as usize;
            assert_eq!(path_length, path.len());
            assert_eq!(&packet[9..9 + path_length], path.as_bytes());
            let attributes_start = 9 + path_length;
            let flags = u32::from_be_bytes(
                packet[attributes_start..attributes_start + 4]
                    .try_into()
                    .unwrap(),
            );
            assert_eq!(
                flags,
                FileAttr::PERMISSIONS.bits(),
                "chmod must omit size, owner, times and extended attributes"
            );
            assert_eq!(packet.len(), attributes_start + 8);
            let actual_permissions =
                u32::from_be_bytes(packet[attributes_start + 4..].try_into().unwrap());
            assert_eq!(actual_permissions, permissions);
        }
    });
}
