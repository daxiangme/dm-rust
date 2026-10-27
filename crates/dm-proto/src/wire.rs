//! 底层读写：小端整数、包头、字符串编码。
use crate::error::{ProtoError, Result};
use bytes::{BufMut, Bytes, BytesMut};

pub const HEADER_LEN: usize = 64;
/// 单条消息最大长度。
pub const MAX_MSG_LEN: usize = 536_870_912;

/// 服务端字符集。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Gb18030,
    EucKr,
}

impl Encoding {
    pub fn from_code(c: i32) -> Self {
        match c {
            1 => Encoding::Utf8,
            2 => Encoding::EucKr,
            _ => Encoding::Gb18030,
        }
    }
    fn rs(&self) -> &'static encoding_rs::Encoding {
        match self {
            Encoding::Utf8 => encoding_rs::UTF_8,
            Encoding::Gb18030 => encoding_rs::GB18030,
            Encoding::EucKr => encoding_rs::EUC_KR,
        }
    }
    pub fn decode(&self, b: &[u8]) -> String {
        match self {
            Encoding::Utf8 => String::from_utf8_lossy(b).into_owned(),
            _ => self.rs().decode(b).0.into_owned(),
        }
    }
    pub fn encode(&self, s: &str) -> Vec<u8> {
        match self {
            Encoding::Utf8 => s.as_bytes().to_vec(),
            _ => self.rs().encode(s).0.into_owned(),
        }
    }
}

/// 请求构造器：前 64 字节为包头，固定字段用 `set_*`（绝对偏移），变长数据用 `push_*`（追加）。
pub struct Writer {
    buf: BytesMut,
}

impl Writer {
    pub fn new(cmd: i16, stmt_id: i32) -> Self {
        let mut buf = BytesMut::with_capacity(256);
        buf.resize(HEADER_LEN, 0);
        buf[0..4].copy_from_slice(&stmt_id.to_le_bytes());
        buf[4..6].copy_from_slice(&cmd.to_le_bytes());
        Writer { buf }
    }
    pub fn set_u8(&mut self, off: usize, v: u8) {
        self.buf[off] = v;
    }
    pub fn set_u16(&mut self, off: usize, v: u16) {
        self.buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }
    pub fn set_i16(&mut self, off: usize, v: i16) {
        self.buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }
    pub fn set_i32(&mut self, off: usize, v: i32) {
        self.buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    pub fn set_i64(&mut self, off: usize, v: i64) {
        self.buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }
    pub fn push_u8(&mut self, v: u8) {
        self.buf.put_u8(v);
    }
    pub fn push_u16(&mut self, v: u16) {
        self.buf.put_u16_le(v);
    }
    pub fn push_i16(&mut self, v: i16) {
        self.buf.put_i16_le(v);
    }
    pub fn push_i32(&mut self, v: i32) {
        self.buf.put_i32_le(v);
    }
    pub fn push_u32(&mut self, v: u32) {
        self.buf.put_u32_le(v);
    }
    pub fn push_i64(&mut self, v: i64) {
        self.buf.put_i64_le(v);
    }
    pub fn push_u64(&mut self, v: u64) {
        self.buf.put_u64_le(v);
    }
    pub fn push_raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    /// i32 长度前缀 + 数据
    pub fn push_i32_bytes(&mut self, b: &[u8]) {
        self.push_i32(b.len() as i32);
        self.push_raw(b);
    }
    /// u16 长度前缀 + 数据
    pub fn push_u16_bytes(&mut self, b: &[u8]) {
        self.push_u16(b.len() as u16);
        self.push_raw(b);
    }
    /// NUL 结尾
    pub fn push_cstr(&mut self, b: &[u8]) {
        self.push_raw(b);
        self.push_u8(0);
    }
    pub fn body_len(&self) -> usize {
        self.buf.len() - HEADER_LEN
    }
    /// 写入 @6 的 body 长度，返回完整明文帧（尚未压缩/加密/校验）。
    pub fn finish(mut self) -> BytesMut {
        let l = (self.buf.len() - HEADER_LEN) as i32;
        self.buf[6..10].copy_from_slice(&l.to_le_bytes());
        self.buf
    }
}

/// 顺序读取器（零拷贝：`bytes()` 返回原缓冲的切片）。
#[derive(Clone)]
pub struct Reader {
    data: Bytes,
    pos: usize,
}

macro_rules! rd {
    ($name:ident, $t:ty, $n:expr) => {
        pub fn $name(&mut self) -> Result<$t> {
            let b = self.take($n)?;
            Ok(<$t>::from_le_bytes(b.try_into().unwrap()))
        }
    };
}

impl Reader {
    pub fn new(data: Bytes, pos: usize) -> Self {
        Reader { data, pos }
    }
    pub fn pos(&self) -> usize {
        self.pos
    }
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        if self.pos + n > self.data.len() {
            return Err(ProtoError::Truncated { at: self.pos, need: n, have: self.data.len().saturating_sub(self.pos) });
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    rd!(u8, u8, 1);
    rd!(u16, u16, 2);
    rd!(i16, i16, 2);
    rd!(i32, i32, 4);
    rd!(u32, u32, 4);
    rd!(i64, i64, 8);
    rd!(u64, u64, 8);
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
    pub fn bytes(&mut self, n: usize) -> Result<Bytes> {
        if self.pos + n > self.data.len() {
            return Err(ProtoError::Truncated { at: self.pos, need: n, have: self.data.len().saturating_sub(self.pos) });
        }
        let b = self.data.slice(self.pos..self.pos + n);
        self.pos += n;
        Ok(b)
    }
    /// i32 长度前缀的字节串
    pub fn i32_bytes(&mut self) -> Result<Bytes> {
        let n = self.i32()?;
        if n < 0 {
            return Ok(Bytes::new());
        }
        self.bytes(n as usize)
    }
    pub fn u16_bytes(&mut self) -> Result<Bytes> {
        let n = self.u16()? as usize;
        self.bytes(n)
    }
    pub fn u8_bytes(&mut self) -> Result<Bytes> {
        let n = self.u8()? as usize;
        self.bytes(n)
    }
    pub fn i32_string(&mut self, enc: Encoding) -> Result<String> {
        Ok(enc.decode(&self.i32_bytes()?))
    }
    pub fn u16_string(&mut self, enc: Encoding) -> Result<String> {
        Ok(enc.decode(&self.u16_bytes()?))
    }
    pub fn u8_string(&mut self, enc: Encoding) -> Result<String> {
        Ok(enc.decode(&self.u8_bytes()?))
    }
    pub fn string_n(&mut self, n: usize, enc: Encoding) -> Result<String> {
        Ok(enc.decode(&self.bytes(n)?))
    }
}

/// 已完整接收并还原（解密/解压/校验）的响应帧。
#[derive(Clone)]
pub struct Frame {
    pub data: Bytes,
}

impl Frame {
    pub fn new(data: Bytes) -> Result<Self> {
        if data.len() < HEADER_LEN {
            return Err(ProtoError::Truncated { at: 0, need: HEADER_LEN, have: data.len() });
        }
        Ok(Frame { data })
    }
    pub fn stmt_id(&self) -> i32 {
        i32::from_le_bytes(self.data[0..4].try_into().unwrap())
    }
    pub fn cmd(&self) -> i16 {
        i16::from_le_bytes(self.data[4..6].try_into().unwrap())
    }
    pub fn ret_code(&self) -> i32 {
        i32::from_le_bytes(self.data[10..14].try_into().unwrap())
    }
    pub fn u8_at(&self, off: usize) -> Result<u8> {
        self.data.get(off).copied().ok_or(ProtoError::Truncated { at: off, need: 1, have: 0 })
    }
    fn arr<const N: usize>(&self, off: usize) -> Result<[u8; N]> {
        self.data
            .get(off..off + N)
            .map(|s| s.try_into().unwrap())
            .ok_or(ProtoError::Truncated { at: off, need: N, have: self.data.len().saturating_sub(off) })
    }
    pub fn u16_at(&self, off: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(self.arr(off)?))
    }
    pub fn i16_at(&self, off: usize) -> Result<i16> {
        Ok(i16::from_le_bytes(self.arr(off)?))
    }
    pub fn i32_at(&self, off: usize) -> Result<i32> {
        Ok(i32::from_le_bytes(self.arr(off)?))
    }
    pub fn u32_at(&self, off: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.arr(off)?))
    }
    pub fn i64_at(&self, off: usize) -> Result<i64> {
        Ok(i64::from_le_bytes(self.arr(off)?))
    }
    /// 从变长区起点（@64）开始读取。
    pub fn body(&self) -> Reader {
        Reader::new(self.data.clone(), HEADER_LEN)
    }
}
