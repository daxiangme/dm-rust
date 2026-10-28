//! 压缩（规格 §5）：算法 id 0 = zlib，1 = snappy。
use crate::proto::error::{ProtoError, Result};
use std::io::{Read, Write};

pub const COMPRESS_ZLIB: u8 = 0;
pub const COMPRESS_SNAPPY: u8 = 1;

/// 压缩 body：返回 `u32 原长度 ‖ 压缩数据`。
pub fn compress_body(body: &[u8], id: u8) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len() / 2 + 8);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    match id {
        COMPRESS_SNAPPY => {
            let c = snap::raw::Encoder::new().compress_vec(body).map_err(|e| ProtoError::Compress(e.to_string()))?;
            out.extend_from_slice(&c);
        }
        _ => {
            let mut e = flate2::write::ZlibEncoder::new(out, flate2::Compression::default());
            e.write_all(body).map_err(|e| ProtoError::Compress(e.to_string()))?;
            out = e.finish().map_err(|e| ProtoError::Compress(e.to_string()))?;
        }
    }
    Ok(out)
}

/// 解压 `u32 原长度 ‖ 压缩数据`。
pub fn decompress_body(data: &[u8], id: u8) -> Result<Vec<u8>> {
    if data.len() < 4 {
        return Err(ProtoError::Compress("compressed body too short".into()));
    }
    let orig = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
    if orig > crate::proto::wire::MAX_MSG_LEN {
        return Err(ProtoError::TooLong);
    }
    let out = match id {
        COMPRESS_SNAPPY => snap::raw::Decoder::new().decompress_vec(&data[4..]).map_err(|e| ProtoError::Compress(e.to_string()))?,
        _ => {
            let mut o = Vec::with_capacity(orig);
            flate2::read::ZlibDecoder::new(&data[4..]).read_to_end(&mut o).map_err(|e| ProtoError::Compress(e.to_string()))?;
            o
        }
    };
    if out.len() != orig {
        return Err(ProtoError::Compress(format!("size mismatch: header {orig}, got {}", out.len())));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let d: Vec<u8> = (0..20000u32).map(|i| (i % 251) as u8).collect();
        for id in [0, 1] {
            let c = compress_body(&d, id).unwrap();
            assert_eq!(decompress_body(&c, id).unwrap(), d);
        }
    }
}
