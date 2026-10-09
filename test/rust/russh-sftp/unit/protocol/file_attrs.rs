use bytes::{BufMut, BytesMut};

use super::*;
use crate::protocol::{File, Name};

fn extension(extended_type: &str, extended_data: &[u8]) -> FileAttributeExtension {
    FileAttributeExtension {
        extended_type: extended_type.to_owned(),
        extended_data: extended_data.to_vec(),
    }
}

#[test]
fn extended_attributes_use_sftp_v3_wire_format_and_preserve_binary_data() {
    let mut attrs = FileAttributes::empty();
    attrs.extended = vec![
        extension("vendor@example.com", &[0, 0xff, b'\n']),
        extension("text@example.com", b"value"),
    ];

    let encoded = crate::ser::to_bytes(&attrs).unwrap();
    let mut expected = BytesMut::new();
    expected.put_u32(FileAttr::EXTENDED.bits());
    expected.put_u32(2);
    expected.put_u32(18);
    expected.put_slice(b"vendor@example.com");
    expected.put_u32(3);
    expected.put_slice(&[0, 0xff, b'\n']);
    expected.put_u32(16);
    expected.put_slice(b"text@example.com");
    expected.put_u32(5);
    expected.put_slice(b"value");
    assert_eq!(encoded, expected.freeze());

    let mut encoded = encoded;
    let decoded: FileAttributes = crate::de::from_bytes(&mut encoded).unwrap();
    assert_eq!(decoded.extended, attrs.extended);
    assert!(encoded.is_empty());
}

#[test]
fn extended_attributes_do_not_consume_the_next_directory_entry() {
    let mut first_attrs = FileAttributes::empty();
    first_attrs.extended = vec![extension("vendor@example.com", &[0xff, 0])];
    let name = Name {
        id: 7,
        files: vec![
            File {
                filename: "first".to_owned(),
                longname: String::new(),
                attrs: first_attrs,
            },
            File::dummy("second"),
        ],
    };

    let mut encoded = crate::ser::to_bytes(&name).unwrap();
    let decoded: Name = crate::de::from_bytes(&mut encoded).unwrap();
    assert_eq!(decoded.files.len(), 2);
    assert_eq!(decoded.files[0].filename, "first");
    assert_eq!(
        decoded.files[0].attrs.extended,
        vec![extension("vendor@example.com", &[0xff, 0])]
    );
    assert_eq!(decoded.files[1].filename, "second");
    assert!(decoded.files[1].attrs.extended.is_empty());
    assert!(encoded.is_empty());
}
