//! DH 密钥交换与对称加密（规格 §4）。
use crate::error::{ProtoError, Result};
use aes::{Aes128, Aes192, Aes256};
use cipher::generic_array::GenericArray;
use cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use des::{Des, TdesEde3};
use md5::{Digest, Md5};
use num_bigint::BigUint;
use rand::RngCore;

const DH_P_HEX: &str = "C009D877BAF5FAF416B7F778E6115DCB90D65217DCC2F08A9DFCB5A192C593EBAB02929266B8DBFC2021039FDBD4B7FDE2B996E00008F57AE6EFB4ED3F17B6D3";
const DH_G: u32 = 5;
/// 固定 IV（取前 block 字节）。
const IV: [u8; 32] = [
    0x20, 0x21, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34,
    0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x20,
];

pub const CIPHER_DES_CFB: i32 = 132;

fn dh_p() -> BigUint {
    BigUint::parse_bytes(DH_P_HEX.as_bytes(), 16).unwrap()
}

/// 客户端 DH 密钥对。
pub struct DhKeyPair {
    x: BigUint,
    y: BigUint,
}

impl DhKeyPair {
    pub fn generate() -> Self {
        let p = dh_p();
        let mut rng = rand::thread_rng();
        let mut buf = [0u8; 64];
        let x = loop {
            rng.fill_bytes(&mut buf);
            let v = BigUint::from_bytes_be(&buf) % &p;
            if v > BigUint::from(1u32) {
                break v;
            }
        };
        Self::from_private(x)
    }
    pub fn from_private(x: BigUint) -> Self {
        let y = BigUint::from(DH_G).modpow(&x, &dh_p());
        DhKeyPair { x, y }
    }
    /// 64 字节大端公钥（左补零）。
    pub fn public_bytes(&self) -> [u8; 64] {
        let b = self.y.to_bytes_be();
        let mut out = [0u8; 64];
        out[64 - b.len()..].copy_from_slice(&b);
        out
    }
    /// 会话密钥：大端最小长度字节串。
    pub fn session_key(&self, server_pub: &[u8]) -> Vec<u8> {
        let sp = BigUint::from_bytes_be(server_pub);
        sp.modpow(&self.x, &dh_p()).to_bytes_be()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Ecb,
    Cbc,
    Cfb,
    Ofb,
    Stream,
}

#[derive(Clone)]
enum Blk {
    Des(Des),
    Des3(TdesEde3),
    A128(Aes128),
    A192(Aes192),
    A256(Aes256),
}

impl Blk {
    fn bs(&self) -> usize {
        match self {
            Blk::Des(_) | Blk::Des3(_) => 8,
            _ => 16,
        }
    }
    fn enc(&self, b: &mut [u8]) {
        match self {
            Blk::Des(c) => c.encrypt_block(GenericArray::from_mut_slice(b)),
            Blk::Des3(c) => c.encrypt_block(GenericArray::from_mut_slice(b)),
            Blk::A128(c) => c.encrypt_block(GenericArray::from_mut_slice(b)),
            Blk::A192(c) => c.encrypt_block(GenericArray::from_mut_slice(b)),
            Blk::A256(c) => c.encrypt_block(GenericArray::from_mut_slice(b)),
        }
    }
    fn dec(&self, b: &mut [u8]) {
        match self {
            Blk::Des(c) => c.decrypt_block(GenericArray::from_mut_slice(b)),
            Blk::Des3(c) => c.decrypt_block(GenericArray::from_mut_slice(b)),
            Blk::A128(c) => c.decrypt_block(GenericArray::from_mut_slice(b)),
            Blk::A192(c) => c.decrypt_block(GenericArray::from_mut_slice(b)),
            Blk::A256(c) => c.decrypt_block(GenericArray::from_mut_slice(b)),
        }
    }
}

#[derive(Clone)]
pub struct Cipher {
    blk: Option<Blk>,
    mode: Mode,
    rc4_key: Vec<u8>,
}

fn need(key: &[u8], n: usize) -> Result<&[u8]> {
    key.get(..n).ok_or_else(|| ProtoError::Crypto(format!("session key too short: need {n}, have {}", key.len())))
}

impl Cipher {
    /// 由算法 id（规格 §4.5）与会话密钥创建。
    pub fn new(id: i32, session_key: &[u8]) -> Result<Self> {
        if id >= 5000 {
            return Err(ProtoError::Unsupported(format!("third-party cipher id {id}")));
        }
        let algo = id & !0x7f;
        let mode = match id & 0x7f {
            1 => Mode::Ecb,
            2 => Mode::Cbc,
            4 => Mode::Cfb,
            8 => Mode::Ofb,
            0 => Mode::Stream,
            m => return Err(ProtoError::Unsupported(format!("cipher mode {m}"))),
        };
        let bad = |e: cipher::InvalidLength| ProtoError::Crypto(e.to_string());
        let blk = match algo {
            0x80 => Some(Blk::Des(Des::new_from_slice(need(session_key, 8)?).map_err(bad)?)),
            0x100 => {
                let mut k = Vec::from(need(session_key, 16)?);
                k.extend_from_slice(&session_key[..8]);
                Some(Blk::Des3(TdesEde3::new_from_slice(&k).map_err(bad)?))
            }
            0x200 => Some(Blk::A128(Aes128::new_from_slice(need(session_key, 16)?).map_err(bad)?)),
            0x400 => Some(Blk::A192(Aes192::new_from_slice(need(session_key, 24)?).map_err(bad)?)),
            0x800 => Some(Blk::A256(Aes256::new_from_slice(need(session_key, 32)?).map_err(bad)?)),
            0x1000 => None,
            a => return Err(ProtoError::Unsupported(format!("cipher algorithm {a:#x}"))),
        };
        if blk.is_none() {
            return Ok(Cipher { blk: None, mode: Mode::Stream, rc4_key: need(session_key, 16)?.to_vec() });
        }
        if mode == Mode::Stream {
            return Err(ProtoError::Unsupported("block cipher without mode".into()));
        }
        Ok(Cipher { blk, mode, rc4_key: vec![] })
    }

    /// 加密；`digest` 为真时追加明文的 MD5（16 字节）。
    pub fn encrypt(&self, plain: &[u8], digest: bool) -> Vec<u8> {
        let mut out = self.crypt(plain, true);
        if digest {
            out.extend_from_slice(&Md5::digest(plain));
        }
        out
    }

    pub fn decrypt(&self, data: &[u8], digest: bool) -> Result<Vec<u8>> {
        if digest {
            if data.len() < 16 {
                return Err(ProtoError::Crypto("ciphertext shorter than digest".into()));
            }
            let (ct, md) = data.split_at(data.len() - 16);
            let plain = self.crypt_dec(ct)?;
            if Md5::digest(&plain).as_slice() != md {
                return Err(ProtoError::Crypto("digest mismatch".into()));
            }
            Ok(plain)
        } else {
            self.crypt_dec(data)
        }
    }

    fn crypt(&self, plain: &[u8], _enc: bool) -> Vec<u8> {
        let Some(b) = &self.blk else { return rc4(&self.rc4_key, plain) };
        let bs = b.bs();
        let iv = &IV[..bs];
        match self.mode {
            Mode::Ecb | Mode::Cbc => {
                // PKCS#7，填充单位 = 分组长度 `[AES 的 ECB/CBC 填充单位待实测]`
                let pad = bs - plain.len() % bs;
                let mut d = plain.to_vec();
                d.extend(std::iter::repeat(pad as u8).take(pad));
                let mut prev = iv.to_vec();
                for ch in d.chunks_mut(bs) {
                    if self.mode == Mode::Cbc {
                        for i in 0..bs {
                            ch[i] ^= prev[i];
                        }
                    }
                    b.enc(ch);
                    prev.copy_from_slice(ch);
                }
                d
            }
            Mode::Cfb => {
                let mut d = plain.to_vec();
                let mut fb = iv.to_vec();
                for ch in d.chunks_mut(bs) {
                    let mut ks = fb.clone();
                    b.enc(&mut ks);
                    for i in 0..ch.len() {
                        ch[i] ^= ks[i];
                    }
                    if ch.len() == bs {
                        fb.copy_from_slice(ch);
                    }
                }
                d
            }
            Mode::Ofb => {
                let mut d = plain.to_vec();
                let mut ks = iv.to_vec();
                for ch in d.chunks_mut(bs) {
                    b.enc(&mut ks);
                    for i in 0..ch.len() {
                        ch[i] ^= ks[i];
                    }
                }
                d
            }
            Mode::Stream => unreachable!(),
        }
    }

    fn crypt_dec(&self, ct: &[u8]) -> Result<Vec<u8>> {
        let Some(b) = &self.blk else { return Ok(rc4(&self.rc4_key, ct)) };
        let bs = b.bs();
        let iv = &IV[..bs];
        match self.mode {
            Mode::Ecb | Mode::Cbc => {
                if ct.is_empty() || ct.len() % bs != 0 {
                    return Err(ProtoError::Crypto("ciphertext not block aligned".into()));
                }
                let mut d = ct.to_vec();
                let mut prev = iv.to_vec();
                for ch in d.chunks_mut(bs) {
                    let cur = ch.to_vec();
                    b.dec(ch);
                    if self.mode == Mode::Cbc {
                        for i in 0..bs {
                            ch[i] ^= prev[i];
                        }
                    }
                    prev = cur;
                }
                let pad = *d.last().unwrap() as usize;
                if pad == 0 || pad > d.len() {
                    return Err(ProtoError::Crypto("bad padding".into()));
                }
                d.truncate(d.len() - pad);
                Ok(d)
            }
            Mode::Cfb => {
                let mut d = ct.to_vec();
                let mut fb = iv.to_vec();
                for ch in d.chunks_mut(bs) {
                    let mut ks = fb.clone();
                    b.enc(&mut ks);
                    let full = ch.len() == bs;
                    let cur = ch.to_vec();
                    for i in 0..ch.len() {
                        ch[i] ^= ks[i];
                    }
                    if full {
                        fb.copy_from_slice(&cur);
                    }
                }
                Ok(d)
            }
            Mode::Ofb => Ok(self.crypt(ct, false)),
            Mode::Stream => unreachable!(),
        }
    }
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: Vec<u8> = (0..=255u8).collect();
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + s[i] as usize + key[i % key.len()] as usize) & 255;
        s.swap(i, j);
    }
    let (mut i, mut j) = (0usize, 0usize);
    data.iter()
        .map(|&b| {
            i = (i + 1) & 255;
            j = (j + s[i] as usize) & 255;
            s.swap(i, j);
            b ^ s[(s[i] as usize + s[j] as usize) & 255]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dh_agreement() {
        let a = DhKeyPair::generate();
        let b = DhKeyPair::generate();
        assert_eq!(a.session_key(&b.public_bytes()), b.session_key(&a.public_bytes()));
        assert_eq!(a.public_bytes().len(), 64);
    }

    #[test]
    fn roundtrip_all() {
        let key: Vec<u8> = (1..=40).collect();
        for id in [132, 0x80 | 1, 0x80 | 2, 0x100 | 4, 0x200 | 4, 0x400 | 8, 0x800 | 2, 0x200 | 1, 0x1000] {
            let c = Cipher::new(id, &key).unwrap();
            for n in [0usize, 1, 7, 8, 15, 16, 17, 100] {
                let p: Vec<u8> = (0..n as u8).collect();
                for dg in [false, true] {
                    let e = c.encrypt(&p, dg);
                    assert_eq!(c.decrypt(&e, dg).unwrap(), p, "id {id} len {n}");
                }
            }
        }
    }

    #[test]
    fn cfb_no_padding_len() {
        let c = Cipher::new(CIPHER_DES_CFB, &[1, 2, 3, 4, 5, 6, 7, 8, 9]).unwrap();
        assert_eq!(c.encrypt(b"abcde", false).len(), 5);
        assert_eq!(c.encrypt(b"abcde", true).len(), 21);
    }

    #[test]
    fn rc4_known_vector() {
        // RFC 6229 风格经典向量：Key="Key", Plain="Plaintext"
        let out = rc4(b"Key", b"Plaintext");
        assert_eq!(out, [0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]);
    }
}
