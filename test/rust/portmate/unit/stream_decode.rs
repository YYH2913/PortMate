use super::*;
#[test]
fn fragmented_unicode_and_invalid_bytes() {
    let mut decoder = StreamDecoder::default();
    let text = "中文🙂\x1b[31m";
    let decoded: String = text
        .as_bytes()
        .iter()
        .map(|byte| decoder.feed(&[*byte]))
        .collect();
    assert_eq!(decoded, text);
    assert!(decoder.finish().is_empty());
    assert_eq!(decoder.feed(&[0xff, b'a', 0xe4]), "�a");
    assert_eq!(decoder.finish(), "�");
    assert_eq!(decoder.feed(b"new"), "new");
}
