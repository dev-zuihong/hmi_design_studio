use crate::{CoreError, CoreResult};

/// Incremental bounded decoder. A serial read is not a protocol frame.
#[derive(Debug)]
pub struct LineDecoder {
    pending: Vec<u8>,
    max_length: usize,
}

impl LineDecoder {
    pub fn new(max_length: usize) -> Self {
        Self {
            pending: Vec::new(),
            max_length,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> CoreResult<Vec<Vec<u8>>> {
        let mut lines = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                lines.push(std::mem::take(&mut self.pending));
            } else if self.pending.len() == self.max_length {
                self.pending.clear();
                return Err(CoreError::new("FRAME_TOO_LARGE", "帧超过配置的长度上限"));
            } else {
                self.pending.push(byte);
            }
        }
        Ok(lines)
    }
}

pub fn crc16_modbus(bytes: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in bytes {
        crc ^= *byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xa001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_split_and_coalesced_lines_and_bounds_partial_line() {
        let mut decoder = LineDecoder::new(8);
        assert!(decoder.push(b"first").unwrap().is_empty());
        assert_eq!(
            decoder.push(b"\nsecond\r\n").unwrap(),
            vec![b"first".to_vec(), b"second".to_vec()]
        );
        assert!(LineDecoder::new(4).push(b"12345").is_err());
        assert_eq!(crc16_modbus(b"123456789"), 0x4b37);
    }
}
