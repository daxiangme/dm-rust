# dm-rust：达梦 DM8 的纯 Rust 异步驱动

按"先协议规格、后独立实现"的方式编写：
`docs/protocol-spec.md`（中文，帧格式 / 命令号 / 握手与加密 / prepare-execute-fetch / 类型编码 / LOB / 事务 / 错误包，
末节为真机实测记录）→ `crates/*` 仅依据该规格实现。**未拷贝任何第三方驱动的代码或注释。**

## 结构

| crate | 内容 |
|---|---|
| `dm-proto` | 无 IO 的协议编解码：帧读写（`Bytes` 零拷贝）、STARTUP/LOGIN/PREPARE/EXEC/FETCH/LOB 等消息、DH + 对称加密（DES/3DES/AES/RC4 × ECB/CBC/CFB/OFB）、zlib/snappy 压缩、XOR/CRC32 校验、值类型编解码（DECIMAL/日期时间/时区/INTERVAL/LOB 头…） |
| `dm-conn` | tokio 异步连接：握手（含 rustls TLS）、登录、语句缓存（LRU）、`query/execute/execute_batch`、`query_multi`（多结果集）、存储过程 `call`（OUT/INOUT/`SYS_REFCURSOR`）、事务、隔离级别、LOB 读写、压缩/整报文加密、取消安全（future 被丢弃后连接标记为 broken） |
| `dm-pool` | 连接池：信号量限流、LIFO 空闲复用、idle/lifetime 过期、取出时 ping、事务中/损坏连接归还时丢弃 |

## 用法

```rust
let opts = dm_conn::ConnectOptions::from_url("dm://SYSDBA:SYSDBA001@127.0.0.1:5236")?;
let mut c = dm_conn::Connection::connect(&opts).await?;
let r = c.query("select ? + 1 from dual", &[1.into()]).await?;

// INTERVAL：IntervalYm / IntervalDt 带前导精度与 qualifier，Display 与服务端 cast 文本一致。
// 绑定到表列时，写入的精度以列定义为准。
let ym = dm_conn::IntervalYm::new(1, 2); // INTERVAL '1-2' YEAR(9) TO MONTH
let dt = dm_conn::IntervalDt::new(3, 4, 5, 6, 789_000_000); // 小数秒单位纳秒，线上精确到微秒
c.execute("insert into t_iv(ym, dt) values(?, ?)", &[ym.into(), dt.into()]).await?;

// 存储过程 / 函数。OUT、INOUT 与 SYS_REFCURSOR 走 call；query/execute 仍只接受 IN。
// `{call ...}` 与 `{?=call ...}` 会在客户端改写成 `begin ... end;`（服务端不接受花括号）。
// `declare ... begin ... end;` 与 `call p(?, ?)` 原样发送。
let out = c.call(
    "begin p_add(?, ?, ?); end;",
    &[dm_conn::Param::In(2.into()), dm_conn::Param::In(3.into()), dm_conn::Param::out_int()],
).await?;
assert_eq!(out.outs[0].as_value(), Some(&dm_conn::Value::Int(5)));

let f = c.call("{?=call f_add(?, ?)}", &[dm_conn::Param::out_int(), dm_conn::Param::In(2.into()), dm_conn::Param::In(3.into())]).await?;
assert_eq!(f.outs[0].as_value(), Some(&dm_conn::Value::Int(5)));

let cur = c.call("begin p_rows(?, ?); end;", &[dm_conn::Param::In(10.into()), dm_conn::Param::out_cursor()]).await?;
let rows = cur.outs[0].as_cursor().unwrap();
assert_eq!(rows.rows.len(), 10);

// 多结果集。一条 SQL 里用分号隔开的多个 SELECT，query_multi 用 NEXT_RESULT 依次取回。
// query / execute 仍然只解释第一份响应。0 行的 SELECT 也占一项。
// 本机 DM8 8.1.4.6 没有 DBMS_SQL.RETURN_RESULT；纯 DML 批只有一条更新计数（最后一条语句的行数）。
let multi = c.query_multi("select 1 as a from dual; select ? as b from dual", &[2.into()]).await?;
assert_eq!(multi.results.len(), 2);
assert_eq!(multi.results[0].as_query().unwrap().rows[0].get_by_name("a"), Some(&dm_conn::Value::Int(1)));
assert_eq!(multi.results[1].as_query().unwrap().rows[0].get_by_name("b"), Some(&dm_conn::Value::Int(2)));
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
