/// UTF-8 is a stream encoding: incomplete scalars must survive transport reads.
#[derive(Default)]
pub(super) struct StreamDecoder {
    pending: Vec<u8>,
}

impl StreamDecoder {
    pub(super) fn feed(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        let mut consumed = 0;
        loop {
            match std::str::from_utf8(&self.pending[consumed..]) {
                Ok(text) => {
                    output.push_str(text);
                    consumed = self.pending.len();
                    break;
                }
                Err(error) => {
                    let end = consumed + error.valid_up_to();
                    output.push_str(std::str::from_utf8(&self.pending[consumed..end]).unwrap());
                    consumed = end;
                    if let Some(length) = error.error_len() {
                        output.push('�');
                        consumed += length;
                    } else {
                        break;
                    }
                }
            }
        }
        self.pending.drain(..consumed);
        output
    }

    pub(super) fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}

#[cfg(test)]
mod tests {
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
}
