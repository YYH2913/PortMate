use thiserror::Error;

use super::{impl_packet_for, impl_request_id, Packet, RequestId};

/// Error Codes for SSH_FXP_STATUS
#[derive(Debug, Error, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum StatusCode {
    /// Indicates successful completion of the operation.
    #[error("Ok")]
    Ok = 0,
    /// Indicates end-of-file condition; for SSH_FX_READ it means that no more data is available in the file,
    /// and for SSH_FX_READDIR it indicates that no more files are contained in the directory.
    #[error("Eof")]
    Eof = 1,
    /// A reference is made to a file which should exist but doesn't.
    #[error("No such file")]
    NoSuchFile = 2,
    /// Authenticated user does not have sufficient permissions to perform the operation.
    #[error("Permission denied")]
    PermissionDenied = 3,
    /// A generic catch-all error message;
    /// it should be returned if an error occurs for which there is no more specific error code defined.
    #[error("Failure")]
    Failure = 4,
    /// May be returned if a badly formatted packet or protocol incompatibility is detected.
    #[error("Bad message")]
    BadMessage = 5,
    /// A pseudo-error which indicates that the client has no connection to the server
    /// (it can only be generated locally by the client, and MUST NOT be returned by servers).
    #[error("No connection")]
    NoConnection = 6,
    /// A pseudo-error which indicates that the connection to the server has been lost
    /// (it can only be generated locally by the client, and MUST NOT be returned by servers).
    #[error("Connection lost")]
    ConnectionLost = 7,
    /// Indicates that an attempt was made to perform an operation which is not supported for the server
    /// (it may be generated locally by the client if e.g. the version number exchange indicates that a required feature is not supported by the server,
    /// or it may be returned by the server if the server does not implement an operation).
    #[error("Operation unsupported")]
    OpUnsupported = 8,
    // PortMate: accept the standard SFTP v4-v6 status range even when version 3 was negotiated.
    #[error("Invalid handle")]
    InvalidHandle = 9,
    #[error("No such path")]
    NoSuchPath = 10,
    #[error("File already exists")]
    FileAlreadyExists = 11,
    #[error("Write protect")]
    WriteProtect = 12,
    #[error("No media")]
    NoMedia = 13,
    #[error("No space on filesystem")]
    NoSpaceOnFilesystem = 14,
    #[error("Quota exceeded")]
    QuotaExceeded = 15,
    #[error("Unknown principal")]
    UnknownPrincipal = 16,
    #[error("Lock conflict")]
    LockConflict = 17,
    #[error("Directory not empty")]
    DirNotEmpty = 18,
    #[error("Not a directory")]
    NotADirectory = 19,
    #[error("Invalid filename")]
    InvalidFilename = 20,
    #[error("Link loop")]
    LinkLoop = 21,
    #[error("Cannot delete")]
    CannotDelete = 22,
    #[error("Invalid parameter")]
    InvalidParameter = 23,
    #[error("File is a directory")]
    FileIsADirectory = 24,
    #[error("Byte range lock conflict")]
    ByteRangeLockConflict = 25,
    #[error("Byte range lock refused")]
    ByteRangeLockRefused = 26,
    #[error("Delete pending")]
    DeletePending = 27,
    #[error("File corrupt")]
    FileCorrupt = 28,
    #[error("Owner invalid")]
    OwnerInvalid = 29,
    #[error("Group invalid")]
    GroupInvalid = 30,
    #[error("No matching byte range lock")]
    NoMatchingByteRangeLock = 31,
    #[serde(other)]
    #[error("Unknown status")]
    Unknown,
}

/// Implementation for SSH_FXP_STATUS as defined in the specification draft
/// <https://datatracker.ietf.org/doc/html/draft-ietf-secsh-filexfer-02#section-7>
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub id: u32,
    pub status_code: StatusCode,
    pub error_message: String,
    pub language_tag: String,
}

impl_request_id!(Status);
impl_packet_for!(Status);

#[cfg(test)]
mod tests {
    use bytes::{BufMut, BytesMut};

    use super::*;

    fn decode_status(code: u32) -> Status {
        let mut encoded = BytesMut::with_capacity(16);
        encoded.put_u32(7);
        encoded.put_u32(code);
        encoded.put_u32(0);
        encoded.put_u32(0);
        let mut encoded = encoded.freeze();
        crate::de::from_bytes(&mut encoded).unwrap()
    }

    #[test]
    fn decodes_standard_extended_status_codes() {
        assert_eq!(decode_status(10).status_code, StatusCode::NoSuchPath);
        assert_eq!(decode_status(11).status_code, StatusCode::FileAlreadyExists);
        assert_eq!(decode_status(18).status_code, StatusCode::DirNotEmpty);
    }

    #[test]
    fn decodes_unknown_status_codes_without_dropping_the_packet() {
        assert_eq!(decode_status(99).status_code, StatusCode::Unknown);
    }
}
