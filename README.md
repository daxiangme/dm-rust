# dm-rust：达梦 DM8 的纯 Rust 异步驱动

按"先协议规格、后独立实现"的方式编写：
`docs/protocol-spec.md`（中文，帧格式 / 命令号 / 握手与加密 / prepare-execute-fetch / 类型编码 / LOB / 事务 / 错误包，
末节为真机实测记录）→ `crates/*` 仅依据该规格实现。**未拷贝任何第三方驱动的代码或注释。**

## 结构

| crate | 内容 |
|---|---|
| `dm-proto` | 无 IO 的协议编解码：帧读写（`Bytes` 零拷贝）、STARTUP/LOGIN/PREPARE/EXEC/FETCH/LOB 等消息、DH + 对称加密（DES/3DES/AES/RC4 × ECB/CBC/CFB/OFB）、zlib/snappy 压缩、XOR/CRC32 校验、值类型编解码（DECIMAL/日期时间/时区/LOB 头…） |
| `dm-conn` | tokio 异步连接：握手（含 rustls TLS）、登录、语句缓存（LRU）、`query/execute/execute_batch`、事务、隔离级别、LOB 读写、压缩/整报文加密、取消安全（future 被丢弃后连接标记为 broken） |
| `dm-pool` | 连接池：信号量限流、LIFO 空闲复用、idle/lifetime 过期、取出时 ping、事务中/损坏连接归还时丢弃 |

## 用法

```rust
let opts = dm_conn::ConnectOptions::from_url("dm://SYSDBA:SYSDBA001@127.0.0.1:5236")?;
let mut c = dm_conn::Connection::connect(&opts).await?;
let r = c.query("select ? + 1 from dual", &[1.into()]).await?;
```

示例：`cargo run -p dm-conn --example basic|tx_batch`，`cargo run -p dm-pool --example pool`。

## 测试

```
cargo test --workspace                      # 离线单测 + 集成测试（连不上 127.0.0.1:5236 时集成测试自动跳过）
DM_TEST_URL=dm://SYSDBA:SYSDBA001@host:5236 cargo test --workspace
RUST_LOG=dm_conn=trace ...                  # 打印原始帧，便于对照规格排错（登录请求只输出命令号和长度）
```

集成测试需要一个 DM8 实例，例如：
`docker run -d --name dm8 -p 5236:5236 -e PAGE_SIZE=16 -e EXTENT_SIZE=32 -e LOG_SIZE=256 -e UNICODE_FLAG=1 -e LENGTH_IN_CHAR=1 -e SYSDBA_PWD=SYSDBA001 <达梦官方 dm8 镜像>`
（镜像仅用于本地兼容性测试，请遵守达梦的开发者许可条款，不要再分发镜像或其中文件。）
