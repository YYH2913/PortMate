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
#[path = "../../test/rust/portmate/unit/stream_decode.rs"]
mod tests;
