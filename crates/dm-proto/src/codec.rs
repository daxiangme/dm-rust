//! 帧编解码：压缩 → 加密 → 校验（规格 §1）。无 IO。
use crate::compress::{compress_body, decompress_body};
use crate::crypto::Cipher;
use crate::error::{ProtoError, Result};
use crate::wire::{Frame, HEADER_LEN, MAX_MSG_LEN};
use bytes::{Bytes, BytesMut};

pub const CMD_STARTUP: i16 = 200;

#[derive(Clone, Default)]
pub struct Codec {
    /// CRC32 校验模式（startup 响应 @45 == 1）
    pub crc: bool,
    /// 0 关 / 1 强制 / 2 自动
    pub compress_mode: u8,
    /// 0 zlib / 1 snappy
    pub compress_id: u8,
    /// 本机连接不自动压缩
    pub local: bool,
    /// 整包加密
    pub cipher: Option<Cipher>,
}

fn xor_check(h: &[u8]) -> u8 {
    h[..19].iter().fold(0u8, |a, b| a ^ b)
}

impl Codec {
    /// 把 `Writer::finish()` 的明文帧变成可发送字节。
    pub fn encode(&self, mut frame: BytesMut) -> Result<Bytes> {
        let cmd = i16::from_le_bytes(frame[4..6].try_into().unwrap());
        let mut body: Vec<u8> = Vec::new();
        let mut body_changed = false;
        let blen = frame.len() - HEADER_LEN;
        if blen > 0 && cmd != CMD_STARTUP {
            let want = self.compress_mode == 1 || (self.compress_mode == 2 && !self.local && blen > 8192);
            if want {
                body = compress_body(&frame[HEADER_LEN..], self.compress_id)?;
                frame[18] = 1;
                body_changed = true;
            }
            if let Some(c) = &self.cipher {
                let src: &[u8] = if body_changed { &body } else { &frame[HEADER_LEN..] };
                body = c.encrypt(src, true);
                body_changed = true;
            }
        }
        if body_changed {
            frame.truncate(HEADER_LEN);
            frame.extend_from_slice(&body);
        }
        let blen = frame.len() - HEADER_LEN;
        if frame.len() > MAX_MSG_LEN {
            return Err(ProtoError::TooLong);
        }
        if self.crc {
            frame[6..10].copy_from_slice(&((blen + 4) as i32).to_le_bytes());
            let c = crc32fast::hash(&frame);
            frame.extend_from_slice(&c.to_le_bytes());
        } else {
            frame[6..10].copy_from_slice(&(blen as i32).to_le_bytes());
            frame[19] = xor_check(&frame);
        }
        Ok(frame.freeze())
    }

    /// 从包头得到需继续读取的 body 字节数（CRC 模式含 4 字节 CRC）。
    pub fn body_len(header: &[u8]) -> Result<usize> {
        let n = i32::from_le_bytes(header[6..10].try_into().unwrap());
        if n < 0 || n as usize + HEADER_LEN > MAX_MSG_LEN + 4 {
            return Err(ProtoError::Invalid(format!("bad body length {n}")));
        }
        Ok(n as usize)
    }

    /// `raw` = 完整接收的 包头 + body（含 CRC）。
    pub fn decode(&self, raw: Vec<u8>, startup: bool) -> Result<Frame> {
        if raw.len() < HEADER_LEN {
            return Err(ProtoError::Truncated { at: 0, need: HEADER_LEN, have: raw.len() });
        }
        let crc = self.crc && !startup;
        let mut total = raw.len();
        if crc {
            if total < HEADER_LEN + 4 {
                return Err(ProtoError::BadChecksum);
            }
            total -= 4;
            let got = u32::from_le_bytes(raw[total..total + 4].try_into().unwrap());
            if crc32fast::hash(&raw[..total]) != got {
                return Err(ProtoError::BadChecksum);
            }
        } else if xor_check(&raw) != raw[19] {
            return Err(ProtoError::BadChecksum);
        }
        let compressed = raw[18] == 1;
        let mut head = raw[..HEADER_LEN].to_vec();
        let mut body = raw[HEADER_LEN..total].to_vec();
        if !startup && !body.is_empty() {
            if let Some(c) = &self.cipher {
                body = c.decrypt(&body, true)?;
            }
            if compressed {
                body = decompress_body(&body, self.compress_id)?;
            }
        }
        head[18] = 0;
        head[6..10].copy_from_slice(&(body.len() as i32).to_le_bytes());
        let mut out = BytesMut::with_capacity(HEADER_LEN + body.len());
        out.extend_from_slice(&head);
        out.extend_from_slice(&body);
        Frame::new(out.freeze())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Writer;

    fn sample(n: usize) -> BytesMut {
        let mut w = Writer::new(5, 7);
        for i in 0..n {
            w.push_u8((i % 13) as u8);
        }
        w.finish()
    }

    fn rt(c: &Codec, n: usize) {
        let enc = c.encode(sample(n)).unwrap();
        let f = c.decode(enc.to_vec(), false).unwrap();
        assert_eq!(f.stmt_id(), 7);
        assert_eq!(f.cmd(), 5);
        assert_eq!(f.data.len(), HEADER_LEN + n);
        if n > 3 {
            assert_eq!(f.data[HEADER_LEN + 3], 3);
        }
    }

    #[test]
    fn xor_mode() {
        rt(&Codec::default(), 100);
        rt(&Codec::default(), 0);
        let enc = Codec::default().encode(sample(10)).unwrap();
        let mut bad = enc.to_vec();
        bad[0] ^= 1;
        assert!(matches!(Codec::default().decode(bad, false), Err(ProtoError::BadChecksum)));
    }

    #[test]
    fn crc_mode() {
        let c = Codec { crc: true, ..Default::default() };
        rt(&c, 50);
        let enc = c.encode(sample(10)).unwrap();
        let mut bad = enc.to_vec();
        bad[66] ^= 1;
        assert!(c.decode(bad, false).is_err());
    }

    #[test]
    fn compress_and_encrypt() {
        let key: Vec<u8> = (1..=32).collect();
        for id in [0u8, 1] {
            let c = Codec {
                compress_mode: 1,
                compress_id: id,
                cipher: Some(Cipher::new(132, &key).unwrap()),
                crc: true,
                ..Default::default()
            };
            rt(&c, 20000);
            rt(&c, 5);
        }
        let c2 = Codec { compress_mode: 2, ..Default::default() };
        let small = c2.encode(sample(100)).unwrap();
        assert_eq!(small[18], 0);
        let big = c2.encode(sample(9000)).unwrap();
        assert_eq!(big[18], 1);
        rt(&c2, 9000);
    }
}
