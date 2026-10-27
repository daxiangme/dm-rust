//! dm-proto：达梦 DM8 线协议编解码（无 IO，基于 `Bytes` 零拷贝）。
//! 依据 `docs/protocol-spec.md` 实现。
pub mod codec;
pub mod compress;
pub mod crypto;
pub mod error;
pub mod msg;
pub mod types;
pub mod value;
pub mod wire;

pub use error::{ProtoError, Result};
pub use value::Value;
