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

/// Demo wire format: UTF-8 JSON, a tab, four hexadecimal CRC-16/MODBUS digits, LF.
pub fn encode_demo_frame(value: &serde_json::Value) -> CoreResult<Vec<u8>> {
    let json = serde_json::to_vec(value)?;
    let crc = crc16_modbus(&json);
    let mut result = json;
    result.extend_from_slice(format!("\t{crc:04X}\n").as_bytes());
    Ok(result)
}

pub fn decode_demo_frame(line: &[u8]) -> CoreResult<serde_json::Value> {
    let split = line
        .iter()
        .rposition(|b| *b == b'\t')
        .ok_or_else(|| CoreError::new("CHECKSUM", "示例协议帧缺少校验和"))?;
    let expected = std::str::from_utf8(&line[split + 1..])
        .ok()
        .filter(|s| s.len() == 4)
        .and_then(|s| u16::from_str_radix(s, 16).ok())
        .ok_or_else(|| CoreError::new("CHECKSUM", "示例协议校验和格式错误"))?;
    if crc16_modbus(&line[..split]) != expected {
        return Err(CoreError::new("CHECKSUM", "示例协议 CRC 校验失败"));
    }
    Ok(serde_json::from_slice(&line[..split])?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_split_and_coalesced_frames() {
        let a = encode_demo_frame(&json!({"id":1,"result":{"value":12}})).unwrap();
        let b = encode_demo_frame(&json!({"id":2,"result":{"value":13}})).unwrap();
        let mut decoder = LineDecoder::new(1024);
        assert!(decoder.push(&a[..7]).unwrap().is_empty());
        let lines = decoder.push(&[&a[7..], &b].concat()).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(decode_demo_frame(&lines[0]).unwrap()["id"], 1);
        assert_eq!(decode_demo_frame(&lines[1]).unwrap()["id"], 2);
    }

    #[test]
    fn rejects_corruption_and_unbounded_partial_frame() {
        let mut frame = encode_demo_frame(&json!({"id":1})).unwrap();
        frame[6] = b'2';
        assert_eq!(
            decode_demo_frame(&frame[..frame.len() - 1])
                .unwrap_err()
                .code,
            "CHECKSUM"
        );
        assert!(LineDecoder::new(4).push(b"12345").is_err());
        assert_eq!(crc16_modbus(b"123456789"), 0x4b37);
    }
}
