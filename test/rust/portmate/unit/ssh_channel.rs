use super::*;

#[test]
fn russh_messages_are_normalized_without_transport_types() {
    assert_eq!(
        SshBackendMessage::from(ChannelMsg::Data {
            data: b"hello".as_slice().into(),
        }),
        SshBackendMessage::Data(b"hello".to_vec())
    );
    assert_eq!(
        SshBackendMessage::from(ChannelMsg::ExitStatus { exit_status: 23 }),
        SshBackendMessage::ExitStatus(23)
    );
    assert_eq!(
        SshBackendMessage::from(ChannelMsg::Eof),
        SshBackendMessage::Eof
    );
}
