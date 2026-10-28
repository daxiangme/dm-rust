# 达梦 DM8 客户端—服务器通信协议规格（v0.1，实现用）

> 本文档只记录**事实**：字段偏移、取值、状态机。来源：对官方 Go 驱动（`gitee.com/chunanyong/dm`，版本 8.1.x）的阅读、对 sqlx-dm 协议文档的交叉核对，以及（阶段 3）对真实 DM8 服务器的实测。
> 不包含任何驱动源码或注释的拷贝。Rust 实现只依据本文档。
> 约定：**所有整数小端**；`u8/u16/u32/i16/i32/i64` 为定宽整数；偏移均相对于**消息起点**（包头第 0 字节）；`@N` 表示偏移 N。
> 标记 `[待实测]` 表示来自源码推断、尚未在真机确认的条目；已确认/修正的内容汇总在 §16。

---

## 1. 传输与帧格式

- 传输：TCP（默认端口 5236），可在其上叠加 TLS（见 §6）。每个请求对应**一个**响应；协议为严格的请求-应答（无多路复用、无流水线）。
- 每条消息 = **64 字节包头** + **变长体（body）**。固定字段区在 `@20..@63`，其后 `@64..` 为变长数据区。有的消息把"固定字段"和"变长数据"都当作 body 的一部分；包头里的 body 长度字段 = 总长度 − 64。

### 1.1 包头（请求与响应相同）

| 偏移 | 类型 | 含义 |
|---|---|---|
| @0  | i32 | 语句句柄 id（stmt id）。请求：目标语句；没有语句的命令填 0。响应：对 alloc 命令返回新 stmt id |
| @4  | i16 | 命令号（§2） |
| @6  | i32 | body 长度（= 消息总长 − 64；若启用 CRC，则含末尾 4 字节 CRC） |
| @10 | i32 | 响应：返回码（0 成功；<0 错误码，错误详情在 body；>0 警告） |
| @14 | i32 | 保留 `[待实测]`（响应中可能含有服务端状态） |
| @18 | u8  | 压缩标志：1 = body 已压缩（§5） |
| @19 | u8  | 校验字节：包头 @0..@18 共 19 字节逐字节异或（未启用 CRC 时）；启用 CRC 时见 §1.2 |
| @20..@63 | — | 各命令的固定字段区（§3 起） |

请求包头中 @10、@14 填 0。

### 1.2 校验

- 默认（startup 响应 @45 的 CRC 标志 ≠ 1）：@19 = XOR(@0..@18)。收发均校验；不匹配视为报文损坏。
- CRC 模式（startup 响应 @45 == 1，登录之前的 startup 消息本身不使用）：body 末尾追加 **4 字节 CRC32**（小端），其值对“包头 64 字节 + 原 body（不含这 4 字节）”计算；包头 @6 的长度字段包含这 4 字节。CRC 为标准 CRC-32（IEEE 802.3，反射，多项式 0xEDB88320，初值与终异或均为 0xFFFFFFFF）`[待实测：CRC 模式下 @19 是否仍写异或字节]`。收到时先校验并剥掉 4 字节，再进入解密/解压。

### 1.3 发送流水线（顺序固定）

1. 构造明文消息（包头 + body，@6 = body 长度）。
2. 若满足压缩条件（§5），压缩 **body**（不含包头），@18 置 1。
3. 若已建立整包加密（§4.3），加密 body（含摘要），更新 @6。
4. 写校验：CRC 或 XOR。
5. 写出（若启用整包 TLS，经 TLS 通道）。
- 消息总长上限 512 MiB（`536870912`）。
- 命令 200（startup）永不压缩、不加密、不走 TLS（TLS 在 startup 之后协商）。body 为 0 时不压缩不加密。

### 1.4 接收流水线

1. 读 64 字节包头；按 @6 读 body（可能为 0）。
2. 校验（CRC 或 XOR）。
3. 若整包加密启用：解密 body 并校验 MD5 摘要（§4.3）；更新 @6。
4. 若压缩：body 前 4 字节为解压后长度（u32），其后为压缩数据；解压后更新 @6。压缩判定：压缩模式 1（强制）→ 恒视为压缩；模式 2（自动）→ 看 @18 == 1；模式 0 → 不压缩。
5. 解析返回码 @10（§12）。

---

## 2. 命令号（包头 @4）

| 值 | 名称 | 说明 |
|---|---|---|
| 1   | LOGIN | 登录 |
| 3   | STMT_ALLOC | 分配语句句柄 |
| 4   | STMT_FREE | 释放语句句柄（要释放的 id 写在包头 @0） |
| 5   | PREPARE_OR_EXEC_DIRECT | 预编译 / 直接执行（由 @21 区分，§8） |
| 6   | EXECUTE_OLD | 旧版执行（服务端版本 ≤ 7.1.2.128 才用，不实现） |
| 7   | FETCH | 取更多行 |
| 8   | COMMIT | 提交 |
| 9   | ROLLBACK | 回滚 |
| 13  | EXECUTE | 执行已预编译语句（单行或批量） |
| 14  | PUT_DATA | 为大对象参数分段上传数据 |
| 29  | LOB_GET_LENGTH | 取 LOB 长度 |
| 30  | LOB_WRITE | 写 LOB |
| 31  | LOB_TRUNCATE | 截断 LOB |
| 32  | LOB_READ | 读 LOB |
| 44  | NEXT_RESULT | 取下一个结果集/多结果 |
| 52  | SET_ISOLATION | 设置事务隔离级别 |
| 71  | GET_TABLE_TS | 取表时间戳（结果集缓存用，不实现） |
| 90  | PRE_EXEC | 大对象参数预执行（§11.3） |
| 91  | EXECUTE_OPT | 常量参数优化执行（不实现） |
| 200 | STARTUP | 启动/协商 |

SQL 类型码（执行响应 @20 的 `retSqlType`）：157 INSERT、158 DELETE、159 UPDATE（统称 DML）、160 SELECT、162 带结果的 DML/ CALL 的结果集型、164 其它、165 设置时区、166 设置隔离级别、178、198（不关心）、149 EXPLAIN、153 SET SCHEMA、147/148/151/152 其它 DDL/控制类、251..256 NLS 格式类设置（251..256 对应 NLS 日期、时间、时间戳、时间戳TZ、时间TZ 格式与日期语言）。

---

## 3. 连接建立总流程

```
TCP connect (NoDelay, KeepAlive)
 └─ STARTUP(200)  ──►  startup response
      ├─ 若 sslEncrypt ∈ {1,4}  → 整条连接升级为 TLS（后续所有消息走 TLS）
      ├─ 若 sslEncrypt == 2      → 仅登录认证用 TLS（握手后丢弃 TLS，只为认证），之后明文
      ├─ 若 startup 返回了加密信息 → 计算 DH 会话密钥，建立 Cipher（§4）
      └─ LOGIN(1)  ──►  login response
 └─ （可选）执行 `set schema "<schema>"`
```

默认客户端：compress=0、loginEncrypt=true（startup 请求里发 DH 公钥）、sslEncrypt 由服务器决定。

## 4. STARTUP(200) 与加密握手

### 4.1 STARTUP 请求

固定区：

| 偏移 | 类型 | 值 |
|---|---|---|
| @20 | i32 | 0 |
| @24 | i32 | 压缩模式 compress（0 关 / 1 强制 / 2 自动） |
| @28 | u8  | 登录加密启用：loginEncrypt ? 1 : 0 |
| @29 | u8  | loginEncrypt ? 2 : 0（含义：期望的加密类别 `[待实测]`） |
| @30 | u8  | 结果集格式：0 = 行存（本驱动使用）；2 = BDTA 列存 |
| @31 | u8  | 压缩算法 id（0 = zlib，1 = snappy） |
| @32 | u8  | 使用登录证书：0 |
| @33 | u8  | 0 |
| @34 | u8  | 1 |
| @35 | u16 | 客户端支持的消息版本 MsgVersion（本驱动发 8，见 §14） |

变长区（自 @64 起顺序）：
1. i32 长度 + 驱动版本字符串（如 `"8.1.4.200"`，服务端编码）；
2. u8 `0`（ukey 标志）；
3. 若 loginEncrypt：i32 长度(=64) + **客户端 DH 公钥**（64 字节，大端，高位补 0）；
4. 若指定 catalog：i32 长度 + 字符串（本驱动不发）。

### 4.2 STARTUP 响应

| 偏移 | 类型 | 含义 |
|---|---|---|
| @20 | i32 | sslEncrypt：0 无；1/4 全程 TLS；2 仅认证 TLS |
| @24 | i32 | 服务器系列号（GlobalServerSeries） |
| @28 | i32 | 服务端字符集：1 = UTF-8；2 = EUC-KR；其它 = GB18030 |
| @32 | i32 | 服务端确认的压缩模式 |
| @36 | i32 | 保留 `[待实测]` |
| @40 | u8  | >0 表示响应里带服务端 DH 公钥 |
| @41 | u8  | >0 表示带加密算法 id；==2 同时带哈希算法 id |
| @42 | u8  | 结果集使用 BDTA（>0） |
| @43 | u8  | 压缩算法 id |
| @44 | u8  | 保留 |
| @45 | u8  | ==1：启用 CRC 校验（§1.2；真机未触发过，见 §16-11） |
| @47 | u8  | ==1：newClientType（登录时 client type 发 5，否则 2） |
| @48 | u16 | 服务端消息版本；客户端取 min(本地, 服务端) 作为会话 MsgVersion |

变长区（**实测：从 @80 起**，@64..@79 为 16 字节保留，见 §16-1；登录响应同理）：
1. 若 MsgVersion < 21：i32 长度 + 服务端版本字符串（点分，如 `8.1.2.xx`；用于推导能力开关，§14）；
2. 若 @41 > 0：i32 加密算法 id；
3. 若 @40 > 0：i32 长度 + 服务端 DH 公钥字节；
4. 若 @41 == 2：i32 哈希算法 id。

### 4.3 加密模式判定

令 `cid` = 响应里的算法 id（缺省 −1），`hasKey` = (@40>0)：
- `hasKey && cid == -1` → **仅登录加密模式**（登录用户名密码加密，之后明文）。算法取 DES-CFB（id 132）。
- `hasKey && cid != -1` → **整包加密模式**：从 LOGIN 起所有消息 body 都加密，算法为 cid。
- 否则不加密。
会话密钥 = `serverPub ^ x mod p`（§4.4），取大端字节串。`cid == -1` 时密码算法按 132。

### 4.4 DH 参数

- P（512 位，十六进制）= `C009D877BAF5FAF416B7F778E6115DCB90D65217DCC2F08A9DFCB5A192C593EBAB02929266B8DBFC2021039FDBD4B7FDE2B996E00008F57AE6EFB4ED3F17B6D3`
- G = 5。
- 客户端私钥 x：随机，0 < x < P；公钥 y = G^x mod P，**序列化为固定 64 字节大端**（不足左补 0）。
- 服务端公钥按无符号大端整数解析；会话密钥 K = serverPub^x mod P，序列化为大端最小长度字节串（无前导 0）。

### 4.5 对称密码

算法 id 位布局：低 7 位 = 工作模式（1 ECB / 2 CBC / 4 CFB / 8 OFB）；高位 = 算法（0x80 DES / 0x100 3DES / 0x200 AES128 / 0x400 AES192 / 0x800 AES256 / 0x1000 RC4）。id ≥ 5000 为外部（第三方）密码，**不支持**。

| 算法 | 密钥取自会话密钥 K |
|---|---|
| DES | K[0..8] |
| 3DES | K[0..16] ‖ K[0..8]（2-key 展开为 24 字节） |
| AES128/192/256 | K[0..16/24/32] |
| RC4 | K[0..16] |

- 固定 IV（取前 blockSize 字节，用于 CBC/CFB/OFB）：`20 21 23 24 25 26 27 28 29 2a 2b 2c 2d 2e 2f 30 31 32 33 34 35 36 37 38 39 3a 3b 3c 3d 3e 3f 20`（32 字节）。
- **每次加密/解密调用都从固定 IV 重新开始**（不延续链状态）；RC4 每次重新以密钥初始化。
- CFB 为全块反馈（DES 64 位 / AES 128 位），OFB 同理；ECB/CBC 需要填充：PKCS#7，**填充单位 = 分组长度**（DES/3DES 为 8，AES 为 16；实测 AES128-ECB/CBC 以 16 为单位通过，见 §16）。 CFB/OFB/RC4 不填充。
- 带摘要变体（整包加密的 body）：密文 ‖ MD5(**明文**)（16 字节）。解密时先拆出末尾 16 字节摘要，解密，再校验 MD5。登录凭证加密不带摘要。
- 默认登录加密算法 DES-CFB（id 132 = 0x80|0x04）。

## 5. 压缩

- 启动协商压缩模式 compress：0 关；1 强制；2 自动。
- 发送条件：命令≠200、body 长度>0，且 (模式 1) 或 (模式 2 且 非本机连接 且 body 长度 > 8192)。
- 压缩后 body = **u32 原始长度** ‖ 压缩数据；@18 置 1；@6 = 4 + 压缩数据长度。
- 压缩算法：id 0 = zlib（含 zlib 头）；id 1 = snappy（块格式，非 framing）。
- 压缩只作用于 body，发生在加密之前；解压发生在解密之后。

## 6. TLS

- sslEncrypt = 1 或 4：startup 响应后，在**同一 TCP 连接**上做 TLS 握手（客户端；服务端证书不校验时 `InsecureSkipVerify`），之后所有消息（含 LOGIN）都经 TLS，且不再启用整包对称加密外的任何差异。
- sslEncrypt = 2：握手时需提供客户端证书+私钥（认证用），握手完成后**丢弃 TLS 层**，后续回到明文 TCP（仅用 TLS 完成认证）。
- 本实现：模式 1/4 用 rustls，**默认校验**证书链与服务器名（系统根证书；`ssl_ca` 指定自定义 CA 时只信任它；`ssl_server_name` 覆盖校验名；`ssl_verify=false` 关闭校验以兼容 DM 默认自签证书）；模式 2 记为未实现。

## 7. LOGIN(1)

### 7.1 请求

固定区：

| 偏移 | 类型 | 含义 |
|---|---|---|
| @20 | i32 | client type：newClientType ? 5 : 2 |
| @24 | i32 | 隔离级别（驱动值映射：-1 默认；0 读未提交；1 读已提交；2 可重复读；3 串行化） |
| @28 | i32 | 语言（0 简中；1 英文；2 繁中） |
| @32 | u8  | 只读（0/1） |
| @33 | i16 | 客户端本地时区（分钟，东八区 = 480） |
| @35 | i32 | 会话超时（秒，0 = 不限） |
| @39 | u8  | mppLocal（0） |
| @40 | u8  | 读写分离标志（0） |
| @41 | u8  | 客户端支持新 LOB 格式（1） |
| @42 | u8  | OS 认证类型（0） |

变长区（顺序）：
1. i32 长度 + **用户名**（服务端编码；仅登录加密模式下为 DES-CFB 密文，无摘要；最大 128 字节明文）
2. i32 长度 + **密码**（同上）
3. i32 长度 + appName
4. i32 长度 + osName（如 `linux`）
5. i32 长度 + 主机名（可空，长度 0）
6. u8 rwStandby（0）
7. u16 长度 + serverOption 字符串（空则 0）

### 7.2 响应

| 偏移 | 类型 | 含义 |
|---|---|---|
| @20 | i32 | MaxRowSize |
| @28 | u8  | DDL 自动提交 |
| @29 | i32 | 会话隔离级别 |
| @33 | u8  | 大小写敏感 |
| @34 | u8  | 反斜杠转义标志 |
| @35 | u16 | 服务器模式（0 普通、1 主、2 备） |
| @37 | u16 | 服务器状态（3 mount、4 open、5 suspend） |
| @39 | u8  | ConstParaOpt |
| @40 | i16 | 数据库时区（分钟） |
| @42 | u8  | ==1 则变长区含备库信息 |
| @43 | u8  | 新 LOB 标志（==1 时 LOB 头为 43 字节扩展格式，§11） |
| @44 | i32 | bufPrefetch |
| @48 | i16 | 密码剩余有效期 |
| @50 | u8  | dscControl |
| @55 | i32 | 服务端版本字符串长度（用于变长区末尾） |

变长区（顺序）：i32+字符串 InstanceName；i32 长度 + Schema（长度 0 且 MsgVersion>0 → 当前 Schema = 大写用户名）；i32+LastLoginIP；i32+LastLoginTime；i32 失败次数；i32 警告 id；i32 宽限期；i32+Guid；i32+DbName；若 @42==1：i32+备库主机、i32 端口、u16 备库数；若剩余字节>0：i64 SessionID；若剩余>0：u8 flag（==1 时使用 Oracle 兼容格式串）；若剩余>0：5 个 **u8 长度**字符串（日期、时间、时间戳、时间戳TZ、时间TZ 格式）；若剩余>0 且 @55>0：长度=@55 的服务端版本字符串（点分）。

登录成功后，客户端会话状态：autoCommit=true（客户端状态，随每个执行请求发送，§8/§9）、TrxStatus、Schema。

## 8. 语句：ALLOC / FREE / PREPARE / EXEC DIRECT

### 8.1 STMT_ALLOC(3)
- 请求 @20 u8 = 1。
- 响应：**@0 i32 = 新 stmt id**；@20 u8 = 是否返回基础列名（==1 则列描述含 base name，§10.1）。

### 8.2 STMT_FREE(4)
- 请求：包头 @0 = 要释放的 stmt id；无 body。响应无附加数据。

### 8.3 命令 5：PREPARE / EXEC DIRECT 请求

包头 @0 = stmt id。固定区：

| 偏移 | 类型 | 含义 |
|---|---|---|
| @20 | u8  | autoCommit（1/0，客户端会话状态） |
| @21 | u8  | **1 = 直接执行**；**0 = 仅预编译** |
| @22 | u8  | 0 |
| @23 | u8  | 1 |
| @24 | u8  | 0 |
| @25 | i16 | 0（执行类型） |
| @27 | i64 | maxRows（无限制用 `i64::MAX`） |
| @35 | u8  | 结果集格式：0 行存 / 2 BDTA（须与 startup 协商一致） |
| @36 | i16 | 0 |
| @38 | u8  | MsgVersion ≥ 3：直接执行=1，预编译=0 |
| @39 | u8  | 0 |
| @40 | u8  | 0 |
| @41 | i32 | 查询超时（秒，0 = 不限） |
| @45 | u8  | innerExec（驱动内部语句，0） |

变长区：SQL 文本，服务端编码，**以 NUL 结尾**。

### 8.4 PREPARE 响应（@21 == 0）

| 偏移 | 类型 | 含义 |
|---|---|---|
| @20 | i16 | retSqlType（§2） |
| @22 | u16 | 参数个数 |
| @24 | i16 | 结果列个数 |
| @26 | i64 | 忽略 |
| @34 | i32 | 事务状态 TrxStatus |

变长区：先 **参数描述 × 参数个数**，再 **列描述 × 列个数**（§10.1）。参数描述（每个）：
`i32 colType, i32 prec, i32 scale, i32 nullable, i16 itemFlag, i32(忽略), i16 ioType, i16 nameLen, i16 typeNameLen, i16 tableNameLen, i16 schemaNameLen`，随后依次是这四个字符串（长度即上述字段，无 NUL）；若 itemFlag&2（LOB）：`i32 lobTabId, i16 lobColId`。`nullable`：0 = 不可空，1 = 可空（§16 第 20 条）。
itemFlag 位：1 自增、2 LOB、4 只读、8 类型仅为服务端推荐、0x100 有默认值。ioType：0 IN、1 OUT、2 INOUT、-1 未知。复杂类型（ARRAY/CLASS/RECORD）在列描述末尾追加类型描述符，本驱动不支持。

## 9. 执行 EXECUTE(13) 及响应

### 9.1 请求

包头 @0 = stmt id。固定区：

| 偏移 | 类型 | 含义 |
|---|---|---|
| @20 | u8  | autoCommit |
| @21 | u16 | 参数个数 |
| @23 | u8  | 1 |
| @24 | i64 | 本次提交的**批量行数**（单行 = 1，无参数也写参数行数 0 `[待实测]`） |
| @32 | i64 | cursorUpdateRow（0） |
| @40 | i64 | maxRows（`i64::MAX` 表示不限） |
| @48 | u8  | MsgVersion≥3：1（参数描述采用扩展布局） |
| @49 | u8  | bit0 = continueBatchOnError；bit7 = multiRowsError |
| @50 | u8  | 0 |
| @51 | u8  | 0 |
| @52 | i32 | 查询超时（0 → 写 −1） |
| @56 | i32 | batchAllowMaxErrors（0） |
| @60 | u8  | innerExec（0） |

变长区（仅当参数个数 > 0）：

1. **参数描述 × N**：`i8 ioType`, `i32 colType`, `i32 prec`, `i32 scale`。ioType 为 0 IN、1 OUT、2 INOUT，与 PREPARE 参数描述一致。colType/prec/scale 取 PREPARE 返回的服务端参数类型。特殊：Oracle 日期掩码时 scale |= 0x2000；本地时区 datetime 时 scale |= 0x1000；Oracle FLOAT 时 prec=round(prec*3.32193)、scale=129。
2. **参数数据 × 批量行数**，每行按参数顺序，每个参数值：
   - NULL：`u16 0xFFFE`
   - 普通值（长度 ≤ 0xFF00 = 65280）：`u16 长度` ‖ 数据
   - 大对象走带外上传（§11.3）：写 `i32 0`
   - LOB 控制值：`u16 0xFFFB` ‖ 控制数据 `[暂不实现]`
   - OUT 参数：不写任何字节（不是 `u16 0xFFFE`）；INOUT 要写输入值。CURSOR / `SYS_REFCURSOR` 同样不写值。写 `i32 4` + cursor stmt id 会在本服务器上被拒绝（见 §16 第 18 条）
   - 超 65280 字节且非 LOB/复杂类型 → 客户端报"数据过长"。

### 9.2 响应（EXEC 与 EXEC DIRECT 通用）

| 偏移 | 类型 | 含义 |
|---|---|---|
| @20 | i16 | retSqlType |
| @22 | u16 | 结果列个数 |
| @24 | i64 | 影响行数 / SELECT 为结果总行数（`i64::MAX` = 未知） |
| @32 | u16 | OUT 参数个数 |
| @34 | u8  | 结果集可更新 |
| @35 | u32 | **本包内携带的行数** |
| @39 | u32 | printLen（服务端 print 输出字节数） |
| @43 | —   | 若 retSqlType ∈ {160,162}：`@43 u8`（==2 → 本包为 BDTA 格式）、`@44 u16`（BDTA 时列数偏移用）、跳过 5 字节（至 @51）；否则：`@43 i64 rowid` |
| @51 | u32 | execId |
| @55 | u32 | rsCacheOffset |
| @59 | u8  | 标志：bit0 = 带批量逐行计数；bit1 = 带自增 id 列表 |
| @60 | u32 | TrxStatus（事务状态，0 = 事务结束，非 0 = 事务进行中） |

变长区顺序：
1. printLen>0：服务端 print 文本；
2. OUT 参数数据，只含 OUT 与 INOUT、按参数顺序（u16 长度；0xFFFE=NULL；0xFFFF → 后跟 i32 实际长度）。CURSOR 在这里占一个 NULL 槽，行在随后的结果集里（见 §16 第 18 条）；
3. 按 retSqlType：
   - 160/162：列描述 × @22（§10.1），然后行数据 × @35（§10.2）；
   - 165：i16 时区；166：i16 隔离级别 + u8 只读；153：i32+字符串 新 Schema；149：i32+字符串 执行计划；
   - 157/158/159（DML）：若 @59 bit0：i32 计数 n + i64×n 逐行影响行数；若 @59 bit1：i32 n + i64×n 自增 id；批量出错时返回码为 EC_BP_WITH_ERROR，body 里附逐行错误：`skip 4; i32 count; (i32 行号, i32 错误码, i16+字符串 错误信息) × count`。
   - 251..256：NLS 设置类，响应带字符串/字节；
4. 若 rsCacheOffset>0：i16 表个数 + (i32 表 id + i64 时间戳)×n（忽略）。

TrxStatus 由 EXEC 响应更新；客户端据此决定 commit 是否有意义。

## 10. 结果集

### 10.1 列描述（每列）

`i32 colType, i32 prec, i32 scale, i32 nullable, i16 itemFlag, i32(忽略), i16(忽略), i16 nameLen, i16 typeNameLen, i16 tableNameLen, i16 schemaNameLen`，随后四个字符串；若 STMT_ALLOC 响应指示有 base name：`i16 长度 + baseName`；若 itemFlag&2（LOB）：`i32 lobTabId + i16 lobColId`。`nullable`：0 = 不可空，1 = 可空（§16 第 20 条）。

itemFlag：1 自增、2 LOB、4 只读。列修饰：
- DATETIME/DATETIME2：scale&0x1000 → 本地时区列（去掉该位）；scale&0x2000 → Oracle DATE 语义（去掉该位）。
- DECIMAL 且 scale==129 → Oracle FLOAT：`prec = round(prec*0.30103)+1`，scale=-1。
- VARCHAR 且 prec==512、scale==6 → BFILE。

### 10.2 行数据（行存格式，非 BDTA）

每行：`u16 行长(忽略) ‖ i64 rowid ‖ u16×列数（各列偏移，忽略） ‖ 各列值`。
每个列值：`u16 len`；`0xFFFE` = NULL；`0xFFFF` = 长值，后接 `i32 len` + 数据；否则 len 字节数据。
（BDTA 列存格式：本驱动不请求，不实现。）

### 10.3 FETCH(7)

请求（包头 @0 = stmt id）：

| 偏移 | 类型 | 含义 |
|---|---|---|
| @20 | i64 | 起始行号（0 基，已取行数） |
| @28 | i64 | 本次期望取行数 |
| @36 | i16 | 结果集 id（首个结果集为 0，之后每多一个结果集 +1） |
| @38 | i32 | 预取缓冲字节数（可写 0 = 服务端默认） |

响应：@20 i64 = 总行数/影响行数；@28 i32 = 本包行数；变长区行数据格式同 §10.2。结果集结束判定：已取行数 ≥ @24 总行数；总行数为 `i64::MAX` 时，以本次返回 0 行为结束。

### 10.4 NEXT_RESULT(44)
请求 @20 i16 = 结果集 id；响应同 EXEC 响应（§9.2），是否有结果集看 retSqlType/列数。

顺序取下一个时 @20 写 **0**。写 **从 1 起的序号**会按绝对位置重读对应结果（1 是第一个）。序号超出实际个数时服务端返回 **-6006（无效消息）**，这是普通错误响应，连接还可以继续用。第一份 EXECUTE 响应里没有“后面还有结果”的标志位（多语句 SELECT 的 @59 为 0），客户端要自己发 NEXT_RESULT，直到结束。

结束与空结果集（实测，见 §16 第 19 条）：
- 没有更多结果：返回码 **+111**（警告，不是失败），sql type = 0，无列。body 是错误包布局，消息为「结果集数据获取完成」。
- 这个结果集本身是 0 行：返回码 **+100**，sql type 仍是 162，列描述还在，行数为 0。这是一项结果，后面的 NEXT_RESULT 仍可能取出后续结果集。

REF CURSOR（实测，见 §16 第 18 条）：EXECUTE 响应只带第一个游标。之后对同一 stmt 发 NEXT_RESULT，@20 写 0 取下一个游标；再写 0 则 retSqlType=0 且无列，表示结束。直接写 1、2 会重新读取对应游标（从 1 起）。取游标后续行时 FETCH 的结果集 id 用 0（当前结果集），起始行号为已经收到的行数。

多条 SELECT 写在同一条 SQL 里（`select ...; select ...`，PREPARE+EXECUTE 与直接执行都一样）时，EXECUTE 只带第一个结果集，其余用 NEXT_RESULT(@20=0) 按顺序取。当前结果集没取完的行用 FETCH，结果集 id 为 0。DML 不会各自成为一项结果。细节见 §16 第 19 条。

## 11. 值编码（服务端类型码与字节表示）

### 11.1 类型码

CHAR 0、VARCHAR2 1、VARCHAR 2、BIT 3、TINYINT 5、SMALLINT 6、INT 7、BIGINT 8、DECIMAL 9、REAL 10、DOUBLE 11、BLOB 12、BOOLEAN 13、DATE 14、TIME 15、DATETIME 16、BINARY 17、VARBINARY 18、CLOB 19、INTERVAL_YM 20、INTERVAL_DT 21、TIME_TZ 22、DATETIME_TZ 23、DATETIME2 26、DATETIME2_TZ 27、NULL 28、ARRAY 117、CLASS 119、CURSOR 120、PLTYPE_RECORD 121、SARRAY 122、UNKNOWN 54。
默认精度：DATE 3、TIME 5、DATETIME 8、DATETIME2 9、TIME_TZ 7、DATETIME_TZ 10、DATETIME2_TZ 11、INTERVAL_YM 12、INTERVAL_DT 24（后两者的 prec 是值字节数；前导精度与 qualifier 在 scale，见 §11.2.3）；VARCHAR/VARBINARY 最大 8188。

### 11.2 各类型字节表示（行数据和参数通用）

| 类型 | 编码 |
|---|---|
| BIT / BOOLEAN / TINYINT | 1 字节（TINYINT 有符号 i8；BIT/BOOLEAN 0/1） |
| SMALLINT | i16 LE |
| INT | i32 LE |
| BIGINT | i64 LE |
| REAL | f32 LE（IEEE-754） |
| DOUBLE | f64 LE |
| CHAR / VARCHAR / VARCHAR2 | 服务端编码字节串（UTF-8 实例即 UTF-8）；CHAR 的 VARCHAR2 补空格逻辑在客户端可选 |
| BINARY / VARBINARY | 原始字节 |
| DECIMAL（XDEC，≤21 字节） | 见 §11.2.1 |
| DATE | 3 字节位压缩（§11.2.2） |
| TIME | 5 字节 |
| TIME_TZ | 7 字节（5 字节时间 + i16 时区分钟） |
| DATETIME | 8 字节 |
| DATETIME2 | 9 字节 |
| DATETIME_TZ | 10 字节（8 + i16 时区） |
| DATETIME2_TZ | 11 字节（9 + i16 时区；读取时区位置 = 长度−2） |
| INTERVAL_YM | 12 字节，小端：i32 年、i32 月、i32 scale（@8）。scale 布局见 §11.2.3 |
| INTERVAL_DT | 24 字节，小端：i32 日、时、分、秒、i32 小数秒（微秒）、i32 scale（@20）。scale 布局见 §11.2.3 |
| BLOB / CLOB | LOB 头，见 §11.3 |

类型根据长度判别时区/精度：解码 DATE/TIME 等时，依 `len(value)` 为 3/5/7/8/9/10/11 判断类型（行存格式）。

#### 11.2.1 DECIMAL（XDEC）
- 首字节 `0x80` 且长度 1 → 零。
- 否则首字节 b0：`b0 & 0x80 != 0` → 正数；否则负数。
- 正数：指数 `e = b0 - 193`；每个后续字节 `d = byte - 1`，为 0..99 的两位十进制数字；遇 d∉[0,99] 结束。
- 负数：`e = 62 - b0`；`d = 101 - byte`；负数编码末尾可能有 `0x66` 结束符。
- 值 = 0.d1d2d3…（以 100 为基）× 100^(e+1)，即：数字串 `D`（每字节 2 位）、权重 `weight = e*2 - (len(D) - 2)`，值 = ±D × 10^weight。
- 编码方向：把十进制绝对值拆成两位一组（需要时在尾补 0 使权重为偶），`exp = (weight + len(D))/2 - 1`；首字节正数 `exp + 193`、负数 `62 - exp`；数字字节正数 `d+1`、负数 `101-d`；负数在所有数字后追加 `0x66`（若未超长）；最多 21 字节。指数范围：正 ≤ 62、负 ≤ 61、下限 −64。

#### 11.2.2 日期时间位压缩（行存）
日期部分（3 字节）：`year = (b0 | b1<<8) & 0x7FFF`（若 >9999 则按 16 位补码取负年，用于公元前）；`month = ((b1>>7)&1) | ((b2&7)<<1)`；`day = (b2>>3)&0x1F`。
时间部分（从偏移 t 起，DATE 无）：
- 对 TIME（t=0）与 DATETIME 系（t=3）：`hour = b[t]&0x1F`；`minute = (b[t]>>5)&7 | (b[t+1]&7)<<3`；`second = (b[t+1]>>3)&0x1F | (b[t+2]&1)<<5`；
- 小数秒：TIME / DATETIME / *_TZ（8 字节型）为 **微秒** = `(b[t+2]>>1)&0x7F | b[t+3]<<7 | (b[t+4]&0x1F)<<15`（×1000 得纳秒）；DATETIME2/DATETIME2_TZ 为 **纳秒** = `(b[t+2]>>1)&0x7F | b[t+3]<<7 | b[t+4]<<15 | (b[t+5]&0x7F)<<23`。
- 时区（TZ 类型）：末尾 i16，单位分钟。
- 编码是上面的逆过程。DATETIME 写入微秒（纳秒/1000）。
- 本地时区列（scale 标志 0x1000）：服务端以数据库时区存储，客户端需按（客户端时区 − 数据库时区）调整；`[待实测，默认实现不调整并在文档标注]`。

#### 11.2.3 INTERVAL 的 scale 与 qualifier

YM 的 @8 与 DT 的 @20 都是一个小端 i32，并且与列描述 / 参数描述里的 `scale` 是同一个字。字节（相对该 i32 的起点）：

| 偏移 | 含义 |
|---|---|
| @0 的高 4 位 | 前导精度 `lead_prec`，合法值 1..=9 |
| @0 的低 4 位 | 小数秒精度 `frac_prec`。含 SECOND 的限定符为 0..=6；其余限定符实测恒为 6 |
| @1 | qualifier（下表） |
| @2..@3 | 0 |

即 `scale = (qualifier << 8) | (lead_prec << 4) | frac_prec`。

| qualifier | SQL | 类型码 |
|---|---|---|
| 0 | YEAR | INTERVAL_YM (20) |
| 1 | YEAR TO MONTH | INTERVAL_YM (20) |
| 2 | MONTH | INTERVAL_YM (20) |
| 3 | DAY | INTERVAL_DT (21) |
| 4 | DAY TO HOUR | INTERVAL_DT (21) |
| 5 | DAY TO MINUTE | INTERVAL_DT (21) |
| 6 | DAY TO SECOND | INTERVAL_DT (21) |
| 7 | HOUR | INTERVAL_DT (21) |
| 8 | HOUR TO MINUTE | INTERVAL_DT (21) |
| 9 | HOUR TO SECOND | INTERVAL_DT (21) |
| 10 | MINUTE | INTERVAL_DT (21) |
| 11 | MINUTE TO SECOND | INTERVAL_DT (21) |
| 12 | SECOND | INTERVAL_DT (21) |

小数秒是**有符号 i32，单位微秒**，与 `frac_prec` 无关：不足 6 位时低位补 0（`0.789` 且 frac_prec=3 存 `789000`；`-0.000001` 存 `-1`）。负间隔的每个分量都 ≤ 0。年月日字段各自独立存放，不互相进位（`INTERVAL '13' MONTH` 的月是 13，年是 0）。`YEAR TO MONTH` 的月必须在 0..=11，否则服务端 -6171。

前导精度：SQL **字面量**省略时为 **9**，**列定义**省略时为 **2**。`lead_prec = 0` 报 -6143；年月 `lead_prec > 9` 报 -6125；日时 `lead_prec > 9` 或 `frac_prec > 6` 报 -6126。单个字段绝对值最大 999999999（超出为 -6115）。`prec` 与这些精度无关，YM 恒为 12，DT 恒为 24。

参数绑定时，值字节里的 scale 必须与参数描述的 scale 相同，否则服务端返回 **-6117（INTERVAL数据溢出）**。绑定到已声明类型的列时，用列的 scale 覆盖值内 scale（数值仍按列精度检查）；`select ?` 这类推荐类型则用值自带的 lead/frac/qualifier。

### 11.3 LOB

**行内的 LOB 值**（新 LOB 格式，登录响应 @43==1）是一个 LOB 头，偏移：

| 偏移 | 类型 | 含义 |
|---|---|---|
| @0  | u8  | 1 = 行内（数据紧跟头部）；2 = 行外 |
| @1  | i64 | blobId |
| @9  | i32 | LOB 长度（字节；CLOB 行内亦为字节数；行外 −1 表示需另查） |
| @13 | i16 | 行外 groupId |
| @15 | i16 | 行外 fileId |
| @17 | i32 | 行外 pageNo |
| @21 | i32 | tableId |
| @25 | i16 | colId |
| @27 | i64 | rowId |
| @35 | i16 | FPA groupId |
| @37 | i16 | FPA fileId |
| @39 | i32 | FPA pageNo |
| @43 | —   | 行内数据起点（长度 = @9） |

旧 LOB 格式（@43==0）：头 13 字节（行内）/21 字节（行外），无 tableId/colId/rowId/FPA 部分，数据在 @13 起。tableId/colId 此时来自列描述 §10.1。

**LOB_GET_LENGTH(29)**：请求 body（自 @64，**追加式**）：`u8 lobFlag(0=BLOB,1=CLOB)`、`u64 blobId`、`i16 groupId`、`i16 fileId`、`i32 pageNo`；新 LOB 格式再追加 `i32 tabId, i16 colId, u64 rowId, i16 exGroupId, i16 exFileId, i32 exPageNo`。响应：@64 起 8 字节则 i64，否则 u32 长度。
**LOB_READ(32)**：请求 body（自 @64）：`u8 lobFlag`、`i32 tabId`、`i16 colId`、`u64 blobId`、`i16 groupId`、`i16 fileId`、`i32 pageNo`、`i16 curFileId`、`i32 curPageNo`、`i32 totalOffset`、`i32 offset(0 基，BLOB 为字节、CLOB 为字符)`、`i32 length`；新格式再追加 `u64 rowId, i16 exGroupId, i16 exFileId, i32 exPageNo`。每次读取长度上限：BLOB 32000 字节、CLOB 16000 字符。响应 body（自 @64）：`u8 readOver(1=读完)`、`u32 dataLen`；若 dataLen>0：`i16 curFileId`、`i32 curPageNo`、`i32 totalOffset`、`dataLen 字节数据`，若仍有剩余：`u32 字符数`（CLOB）。
**LOB 参数写入**：LOB 参数 ≤ 2048 字节时作为普通 u16 长度值内联；> 2048 字节时走带外：先发 PRE_EXEC(90)，再对每个大参数循环 PUT_DATA(14)（每块 ≤ 16000 字节），最后 EXECUTE 中该参数位置写 `i32 0`。PUT_DATA 请求：@20 i16 参数序号；body：`i32 块长度`、（新 LOB 格式）`i32 -1`、块数据。 **已实测**（BLOB/CLOB，3 000 B ~ 300 KB）。细节见 §16 第 6 条：PRE_EXEC(90) 的请求体格式、EXECUTE 中带外占位为 **u16 0**（不是 i32 0）、LOB 读取的初始游标。

## 12. 返回码与错误包

- 包头 @10 = 返回码 `code`。`code < 0` 且 `code ≠ EC_RN_EXCEED_ROWSET_SIZE` → 错误；`code > 0` 为警告（请求仍然成功，帧要照常解析）；`EC_RN_EXCEED_ROWSET_SIZE` 是“需要继续 fetch”的信号，不是错误。多结果集上两个警告有固定含义：`+100` 是空结果集，`+111` 是没有更多结果集（§10.4、§16 第 19 条）。其它正数没有单独解释。
- 错误包 body（自 @64）：`i32 n1`+跳过 n1 字节；`i32 n2`+跳过 n2 字节；`i32 n3`+跳过 n3 字节；然后 `i32 长度`+**错误消息字符串**（服务端编码；EUC-KR 在中文环境下按 GB18030 解码）。`[待实测：三段跳过区在真实服务器的内容]`
- 常见错误码（节选，负数；以实测为准，见 §16 第 20 条）：-2007 语法错误、-2106 无效对象、-2501 用户名或密码错误、-2723 未打开 IDENTITY_INSERT 时给自增列赋值、-6602 唯一性约束、-6609 非空约束。
- 批量错误码 `EC_BP_WITH_ERROR`（§9.2）。

## 13. 事务

- 自动提交是**客户端状态**：每条 PREPARE/EXEC/EXEC DIRECT 请求在 @20 携带 autoCommit。
- 开始事务：客户端把 autoCommit 置 0（无网络消息）。隔离级别变化用 SET_ISOLATION(52)：请求 @20 i32 = 映射后的级别（0 读未提交 / 1 读已提交 / 2 可重复读 / 3 串行化）。只读事务用 SQL `SP_SET_SESSION_READONLY(1)`（以及结束时 `(0)`）。
- COMMIT(8)、ROLLBACK(9)：无 body，无额外响应字段；成功后客户端把 autoCommit 恢复为连接默认。
- 事务结束判定：EXEC 响应 @60 `TrxStatus == 0` 表示当前无活动事务。
- DDL 在 DM 中隐式提交（登录响应 @28 指示）。

## 14. 版本与能力

- 客户端 MsgVersion = min(本地 8, 服务端响应 @48)。
- 服务端版本字符串（MsgVersion<21 时在 startup，或在登录响应末尾）用于开关：`>` 7.1.2.128 才支持命令 13（EXECUTE）；其它开关本驱动不实现。
- 版本字符串小于 `7.1.0.0` 视为不支持。

## 15. 字符编码

- 服务端字符集由 startup @28 给出。UTF-8 实例（UNICODE_FLAG=1）全程使用 UTF-8；GB18030 实例使用 GB18030（`encoding_rs`，已实测，见 §16 第 21 条）。
- 所有字符串字段（SQL、用户名、密码、错误信息）均按该编码。

## 16. 实测记录（DM8 8.1.4.6，Docker 镜像 dm8_20241022_rev244896，UTF-8，PAGE_SIZE=16）

以下条目由真机交互确认；与上文冲突时以本节为准（上文相关位置已就地修订）。

1. **STARTUP / LOGIN 响应的变长区从 @80 开始**（@64..@79 为 16 字节固定扩展，内容全 0 / 保留），不是 @64。其它命令的响应变长区仍从 @64 起。
2. **EXECUTE 响应的列描述**：
   - 响应 @22 列数为 0 时，沿用 PREPARE 响应给出的列描述。
   - 语句首次执行、计划被重新编译等情况下，响应 @22 列数 > 0 并携带完整列描述（此时表达式类型已解析，例如 `select ? + 1` 的列类型在 PREPARE 阶段为 54 UNKNOWN，EXEC 响应里才是 7 INT）。客户端必须用响应中的列描述并**更新缓存的语句元数据**，否则再次执行同一缓存语句会拿到 UNKNOWN 类型。
3. **结果集总行数与 FETCH 终止**：
   - 查询的 EXECUTE 响应 @24（总行数）通常为 `i64::MAX`（未知），响应只带第一包行（实测每包 ≤ 100 行，受包大小限制）。
   - FETCH 响应 @20 为更新后的总行数；`start` 取"已收到的行数"，`count` 取期望行数（实测 1000 即可），@38 预取字节数填 0 可用。
   - 游标取尽后再发 FETCH，服务端返回错误码 **-2002（执行未准备SQL语句）**，应当视为"结果集结束"而不是失败。
4. **批量执行**：任意一行出错时，响应返回码为 **+121（EC_BP_WITH_ERROR，正数）**，body 为逐行错误列表（行号 + 错误码 + 消息）。批量 1 ~ 5000 行均通过测试。
5. **DECIMAL 参数类型**：EXEC 的参数类型声明 prec 必须满足 `1 ≤ scale ≤ prec ≤ 38`（`0.0001` 不能声明为 prec=1, scale=4，否则 -6119 无效的数据类型）；负标度（如 1e10）声明 scale=0、prec 增加相应位数。数值超出目标列精度返回 -6149。
6. **LOB 写入与读取**：
   - ≤ 2048 字节的 LOB 参数按普通值内联（u16 长度 + 数据）。
   - > 2048 字节的 BLOB/CLOB（仅限单行执行）：先 **PRE_EXEC(90)**：@20 i16 参数个数，@22 u8 0，body 为每个参数 13 字节（u8 ioType=0, i32 类型, i32 prec, i32 scale），然后对每个大参数发若干 **PUT_DATA(14)**（块 ≤ 16000 字节，`@20 i16 参数序号`，body = i32 块长 + [新 LOB 格式再加 i32 -1] + 数据），最后发 EXECUTE，大参数位置写 **u16 0**（2 字节）作为带外占位。
   - 读取 LOB（新 LOB 格式，登录响应 @43 == 1）：行里是 43 字节 LOB 头（行内数据紧随其后）。LOB_READ(32) 的游标初值必须是 `(cur_file_id = 头.file_id, cur_page_no = 头.page_no, total_offset = 0)`；全 0 初值会报 -4563 无效的页。CLOB 的 offset 以字符计。
7. **过期语句计划**：对缓存的语句（如 `drop table X`）在表被重建后再执行，服务端返回 **-7009（对象定义被修改）**；释放该语句重新 PREPARE 后重试即可。
8. **整报文加密**（STARTUP 响应 cipher id 不是 -1 时）：已用 `COMM_ENCRYPT_NAME` 配置实测：AES128_CBC(514)、AES128_ECB、AES192_OFB(1032)、AES256_CFB、DES_ECB、3DES_CBC、DES3_CFB、RC4 均登录并通过全部集成测试；hash id 4352 对应请求/响应尾部追加的 MD5 摘要。ECB/CBC 填充单位 = 分组长度。SM4（cipher id 66）等国密算法未实现，客户端应在握手阶段明确报错。仅凭据加密（cipher id = -1，默认）用 DES-CFB(132) 加密用户名/密码。
9. **TLS**：`ENABLE_ENCRYPT = 1` 与 `= 4` 实测通过：STARTUP 响应 @20（sslEncrypt）为 1/4 时，在同一 TCP 连接上立即做 TLS 握手（自签证书，服务端不要求客户端证书），之后的 LOGIN 等全部走 TLS 通道；DH/凭据加密仍照常。模式 2（仅认证）、3（国密 SSL）未支持。
10. **压缩**：`MSG_COMPRESS_TYPE = 2`（默认，自动）。客户端在 STARTUP 请求中选择模式/算法，服务端响应协商结果：mode 1（总是压缩）下 zlib(0) 与 snappy(1) 均通过测试（含 2000 行结果集）；mode 2 在 127.0.0.1 上协商后不实际压缩（客户端按"本地连接"规则跳过）。接收端是否解压只看响应包头 @18。
11. **校验**：`COMM_VALIDATE`（默认 1）是对 @19 异或校验字节的服务端校验开关，不是 CRC 开关；将其设为 2 后 STARTUP 响应 @45 仍为 0。**CRC32 模式未能在真机触发，尚未验证**（仅有离线单测）。
12. **响应解析容错**：响应 body 末尾可能有未识别的多余字节（实测存在），解析器应按字段顺序读取而不要求恰好消耗完整个 body。
13. **登录 schema**：登录请求不带 schema 时，服务端会话的当前模式为用户名同名模式；客户端本地记录的"当前模式"在为空时取 `upper(user)`。
14. **错误包**：布局（三段跳过区 + 长度前缀的消息）与 §12 一致；消息为服务端字符集（UTF-8 实例下为 UTF-8 中文）。登录失败返回的错误码与普通错误包同格式。
15. **已验证的值类型往返**：TINYINT/SMALLINT/INT/BIGINT、FLOAT/DOUBLE、DECIMAL（含 38 位）、CHAR/VARCHAR（含中文）、BINARY/VARBINARY、DATE/TIME/TIMESTAMP、TIME/TIMESTAMP WITH TIME ZONE、BLOB/CLOB、BIT/BOOLEAN、NULL、INTERVAL YEAR TO MONTH 与 INTERVAL DAY TO SECOND（全部 qualifier，含精度、负值、NULL、绑定参数）。本地时区列未专门测试。
16. **未验证**：CRC32 模式、EUC-KR 实例、BDTA（服务端要求时直接报错）、复杂类型（ARRAY/记录/类）、LOB 的 OUT 参数、DBMS_OUTPUT、DM 集群/读写分离、SM4/RSA 等扩展加密、IPv6。错误包里三段跳过区的内容仍是 `[待实测]`。外键约束与 CHECK 约束的错误码这次没有测。INTERVAL 的 qualifier 与 scale 布局已核实，见第 17 条。存储过程 OUT/INOUT、函数返回值与 `SYS_REFCURSOR` 已核实，见第 18 条。一条 SQL 里的多个 SELECT、以及本版本能否用 `DBMS_SQL.RETURN_RESULT` 造出隐式结果集，已核实，见第 19 条。列/参数 `nullable`、语法/登录/唯一/非空错误码、`IDENTITY` 回写，以及事务内的 SAVEPOINT SQL，已核实，见第 20 条。
17. **INTERVAL 值编码**（§11.2.3 的实测依据，DM8 8.1.4.6）：
    - `INTERVAL '1-2' YEAR TO MONTH` 的 12 字节为 `01 00 00 00  02 00 00 00  96 01 00 00`（年=1，月=2，scale=0x0196=406：lead=9，frac=6，qualifier=1）。`INTERVAL '-1-2' YEAR TO MONTH` 的年、月均为负数（`ff ff ff ff  fe ff ff ff`），scale 不变。
    - `INTERVAL '1' YEAR(2)` 的 scale 低字节为 `0x26`（lead=2，frac=6，qualifier=0）；`MONTH(4)` 为 qualifier=2。列 `interval year to month`（未写精度）的 scale=0x0126=294（lead=2；字面量默认 lead=9）。
    - `INTERVAL '3 04:05:06.789' DAY TO SECOND` 的 24 字节为日/时/分/秒/微秒五段 i32：`3, 4, 5, 6, 789000`，随后 `96 06 00 00`（scale=0x0696=1686，qualifier=6）。`INTERVAL '-3 04:05:06.789' DAY TO SECOND` 五段全为负。`INTERVAL '-0.000001' SECOND` 的秒为 0、微秒为 -1。
    - qualifier 0..12 与 YEAR、YEAR TO MONTH、MONTH、DAY、DAY TO HOUR、DAY TO MINUTE、DAY TO SECOND、HOUR、HOUR TO MINUTE、HOUR TO SECOND、MINUTE、MINUTE TO SECOND、SECOND 一一对应。`SECOND(2,6)` 的 scale 低字节为 `0x26`；`DAY(4) TO SECOND(3)` 为 `0x43`；`SECOND(5,0)` 为 `0x50`。
    - 表列往返：插入绑定参数后读回的 scale 等于列描述的 scale（prec 仍是 12 或 24）。值内 scale 与参数描述不一致时返回 -6117。`lead_prec` 超出 1..=9 或 `frac_prec` > 6 在 SQL 层被拒绝，没有额外的线上变体。scale 的高 16 位在全部样本中为 0。
18. **存储过程 / 函数的 OUT、INOUT 与 SYS_REFCURSOR**（DM8 8.1.4.6）：
    - **调用文本**。服务端接受 `begin p(?, ?); end;`、`call p(?, ?)`、`declare ... begin ... end;`，以及 `begin ? := f(?, ?); end;`（函数返回值是第一个 OUT）。JDBC 转义 `{call p(?)}` 与 `{?=call f(?)}` 会报 **-2007**（语法错误，靠近 `call`）。客户端把这两种转义改写成上面的匿名块再发送。
    - **PREPARE**。`a in int, b out int, c in out int`：三者 colType 都是 7 INT、prec=4（字节数，不是十进制位数）、scale=0。IN 的 itemFlag=8（类型仅推荐）、ioType=0；OUT 与 INOUT 的 itemFlag=0，ioType 分别是 1 和 2。`begin ? := f(?, ?); end;` 的第一个参数是 OUT INT（prec=4，名字为空），后面两个是 IN。`out sys_refcursor`：colType=120、prec=32768（0x8000）、scale=0、ioType=1，类型名 `CURSOR`。`in varchar(100)` 这次 PREPARE 给出的 prec 是 32767（0x7FFF），不是声明的 100；`out varchar(200)` 的 prec 是 200；DATE 的 colType=14、prec=3、scale=0。无局部变量的 `begin ? := ? + 5; end;`，PREPARE 给出两个 INT、prec=10（先 OUT 后 IN）。`declare v int; begin v := ?; ? := v + 5; end;` 的两个 INT prec 都是 4（先 IN 后 OUT）。两种都能往返。
    - **EXECUTE 请求**。ioType 必须与 PREPARE 的 0/1/2 一致。把真实 OUT 按 ioType 0 发送并写入值，曾导致 dmserver **SIGSEGV（signal 11）**，会话被拆掉；方向不一致时客户端不得发送 EXECUTE。每个参数先写 13 字节描述（`u8 ioType, i32 colType, i32 prec, i32 scale`），再按顺序写值：IN 与 INOUT 写 `u16 长度 + 字节` 或 NULL 的 `u16 0xFFFE`；OUT 与 CURSOR **省略值字节**（不是 NULL 标记，也不是 `i32 4` + stmt id）。后者会得到 `cmd 13 validate error!` 并断开连接。示例（IN=3、OUT INT、INOUT=4、IN 字符串 `你好`、OUT VARCHAR、INOUT `xy`、IN/INOUT 日期 2024-02-28、OUT INT）的值区是 `04 00 03 00 00 00  04 00 04 00 00 00  06 00 e4 bd a0 e5 a5 bd  02 00 78 79  03 00 e8 07 e1  03 00 e8 07 e1`。三个 CURSOR 的请求在描述之后只有 IN 的 `04 00 78 00 00 00`（整数 120），没有游标载荷。
    - **EXECUTE 响应**。retSqlType=162，@22 列数在没有游标时为 0，@32 是 OUT+INOUT 的个数（不含纯 IN）。值区按这个顺序，编码与 §11.2 相同。上面那次调用的 body 是 `04 00 0d 00 00 00`（13）、`04 00 07 00 00 00`（7）、`0a 00` + UTF-8 `你好-out`、`05 00` + `xy-io`、`03 00 e8 07 e9`（2024-02-29）、同样的日期、`fe ff`（NULL）。IN 为 NULL 的 INT/VARCHAR/DATE INOUT，以及被赋成 NULL 的 OUT，响应都是 `fe ff`。函数 `f(20, 22)` 的返回值 body 是 `04 00 2a 00 00 00`（42），@32=1；`f(NULL, 1)` 的 body 只有 `fe ff`。
    - **REF CURSOR**。每个游标在 OUT 区占一个 `fe ff`。第一个游标的列描述和行紧跟在这些 OUT 值后面（retSqlType=162，@22 为列数，@35 为本包行数，@24 为总行数）。120 行、两列（INT + 字符串）的游标整包都在 EXECUTE 响应里：@24=120，@32=3，@35=120，body 以 `fe ff fe ff fe ff` 开始，然后是列描述和行。标量 OUT 与 CURSOR 在同一次调用里时，标量仍是长度前缀的值，游标槽仍是 `fe ff`：INT 结果 9 加一个游标时，body 以 `04 00 09 00 00 00 fe ff` 开始。第二个、第三个游标不在这个包里，要对同一 stmt 发 **NEXT_RESULT(44)**，@20=0。第二包：sql type 162、列数 1、@24=3、@32=0、@35=3，列名 `ID`、类型名 `INTEGER`。空游标（`open c for select 1 as id from dual where 2 = 3`）：sql type 162、1 列、@24=0、@35=0，只有列描述、没有行。两个游标都取完后再发 NEXT_RESULT(@20=0)，响应 sql type=0 且无列。FETCH 的结果集 id 用 0 表示当前游标，起始行号是已经收到的行数；这次 120 行没有拆包。另一次探测里，NEXT_RESULT 之后用 FETCH start=40、rs id=0 能继续读到当前游标的后续行；对已经整包返回的游标再 FETCH、start 超出总行数时，得到 **-7036** 且 body 为空，按结果集结束处理（查询游标耗尽时的 -2002 见第 3 条）。
    - **未在这次确认的**：PREPARE ioType=-1 的真实语句、LOB OUT、把游标按 stmt id 编码的任何成功变体。一条 SQL 中的多个 SELECT 见第 19 条。
19. **多结果集与 NEXT_RESULT(44)**（DM8 8.1.4.6）。第一份 EXECUTE / 直接执行响应里看不到“还有下一个”的标志；@59 在这些 SELECT 批上是 0。客户端对同一 stmt 发 NEXT_RESULT，@20=0 表示下一个，直到结束。
    - **多条 SELECT**。`select 1 as a from dual; select 2 as b, '雪' as c from dual; select 3 as d from dual where 1=0`：PREPARE 的 sql type=162、0 列、0 参数。EXECUTE 只带第一项（sql type 162、列 `A`、值 1）。第一次 NEXT_RESULT(@20=0) 是列 `B`/`C`、值 2 与 `雪`。第二次是空结果集：返回码 **+100**、sql type 162、1 列、@24=0、@35=0，没有行。第三次返回码 **+111**、sql type 0、无列，body 按 §12 的错误包布局，消息「结果集数据获取完成」。`+100` 和 `+111` 都是正数警告，连接保持可用。直接执行（命令 5、@21=1）与 PREPARE 再 EXECUTE 的拆分方式相同。单条 `select 1 as a from dual` 的 EXECUTE sql type 是 160，紧接着的 NEXT_RESULT(@20=0) 就是 +111。
    - **参数**。`?` 按整段文本从左到右编号。`select ? as a from dual; select ? as b from dual` 绑定 7、8，两个结果集分别是 7 和 8。只给第一句占位符时，`select ? as a from dual; select 9 as b from dual` 绑定 7，第二项是字面量 9。
    - **空结果集不是结束**。`select 1 as a from dual where 1=0; select 2 as b from dual` 的第一包就是 +100、有列、0 行；下一次 NEXT_RESULT 才是值为 2 的结果集。
    - **DML 不拆开**。`insert ...; update ...; delete ...` 三条都执行了（插入的行能再被更新、再被删掉），但响应只有一包：sql type 162、0 列、0 行，@24 等于**最后一条**语句的影响行数（这次 delete 删 1 行，@24=1；不是三条之和 5，也不是中间那条 update 的 3）。再 NEXT_RESULT 就是 +111。`select ...; update ...; select ...` 与 `select ...; insert ...` 只交出 SELECT 结果集，DML 的影响行数不单独出现；数据修改仍然发生。存储过程里连续三条 DML 同样只有这一个计数（再调用一次、最后的 delete 影响 0 行时，@24=0）。`begin null; end;` 是 sql type 162、0 列、@24=0，然后 +111。
    - **批量参数不是多项结果**。`insert` 两行参数的 EXECUTE：sql type 157，@24=2，@59=0x87。bit0 与 bit1 仍是逐行计数 `[1, 1]` 和自增 id；bit2 与 bit7 也是 1，按 §9.2 只读 bit0/bit1 就能得到这两段列表，没有再多出要读的字段。紧接着的 NEXT_RESULT(@20=0) 是 +111，不会按参数行再给结果。
    - **FETCH**。本实例 `LENGTH_IN_CHAR=0`。`rpad('汉', 400, '汉')` 的 `LENGTH` 是 200、`LENGTHB` 是 600，不是 400 个字符。第二项这样的字符串 × 80 行：NEXT_RESULT 包 @24=80、@35=52。随后 FETCH，结果集 id=0、start=52，返回总行数 80、本包 28 行。再 NEXT_RESULT 才是 +111。行数较少的项（180 行、250 行的整数列）整包返回，@24=@35。
    - **序号重读**。多语句 SELECT 执行之后，NEXT_RESULT @20=2 直接给出第二项，@20=1 给出第一项。只有两项时 @20=3 返回 **-6006（无效消息）**。同一连接上随后的其它语句仍然成功。单条 SELECT（sql type 160）在已经收到 +111 之后再写 @20=1，这次仍是 +111，没有把那一行再送出来。顺序消费只用 @20=0。
    - **不取后续结果**。同一 stmt 上 EXECUTE 之后不发 NEXT_RESULT，再次 EXECUTE 仍从第一项开始（值正确）。期间另一条 `select 99 from dual` 也正常。所以 `query` / `execute` 可以只解释第一包。
    - **`DBMS_SQL.RETURN_RESULT` 在这个版本不存在**。`ALL_PROCEDURES` 里 `DBMS_SQL` 有 PARSE、EXECUTE、FETCH_ROWS、TO_REFCURSOR、TO_CURSOR_NUMBER、BIND_VARIABLE、DEFINE_COLUMN 等，没有 `RETURN_RESULT`。匿名块 `open c for select 1 as a from dual; dbms_sql.return_result(c);` 返回 **-2193（无效的方法名[RETURN_RESULT]）**。`create procedure` 把这段正文放进去时，协议层执行成功，PRINT 文本是同一条 -2193，对象处于无效状态；再 `begin` 调用返回 **-7160（对象处于无效状态）**。这个版本用隐式结果集这条路径产不出多结果集。显式 `SYS_REFCURSOR` OUT 参数仍按第 18 条。
20. **nullable、错误码、IDENTITY 与 SAVEPOINT**（DM8 8.1.4.6）：
    - **nullable**。`create table t(id int not null primary key, s varchar(20), n int)` 之后，`select id, s, n` 的列描述与 `insert into t(id, s, n) values(?, ?, ?)` 的参数描述一致：`ID` / 对应 INT 参数 `nullable=0`，`S`（VARCHAR）与 `N`（INT）`nullable=1`。未出现 0/1 以外的值。
    - **错误码**（普通错误包，负数）。`selec 1` 为 **-2007**（第 1 行附近语法错误）。不存在的表为 **-2106**。错误密码与不存在的用户都是 **-2501**，消息为「用户名或密码错误」。主键或 UNIQUE 列重复为 **-6602**（违反唯一性约束）。`NOT NULL` 列不给值时为 **-6609**（违反非空约束）。外键与 CHECK 未测，不要把 -2501 当成语法错误，也不要把 -6602 当成登录失败。
    - **IDENTITY**。`id int identity(1,1) primary key` 插入一行（只写其它列）后，EXECUTE 响应的影响行数是 1，自增 id 列表是 `[1]`（响应里带了自增 id 标志时客户端才填这个列表）。即使写了列清单，`insert into t(id, s) values(1, 'b')` 在未执行 `SET IDENTITY_INSERT t ON` 时返回 **-2723**，消息为「仅当指定列列表，且SET IDENTITY_INSERT为ON时，才能对自增列赋值」。这不是唯一约束。
    - **SAVEPOINT 是 SQL，不是新命令号**。协议事务（`begin` 只把 auto_commit 置 0，`commit` / `rollback` 发 CMD_COMMIT=8 / CMD_ROLLBACK=9）里面可以执行 `SAVEPOINT`、`ROLLBACK TO SAVEPOINT`、`RELEASE SAVEPOINT`。一次测量：表里已有已提交的 1 行；事务内再插入 1 行、`SAVEPOINT`、再插入 1 行、`ROLLBACK TO SAVEPOINT` 之后 `count(*)` 为 2（已提交的 1 行 + 保存点之前的那一行）；`RELEASE SAVEPOINT` 成功；协议 `rollback` 之后 `count(*)` 回到 1。
21. **GB18030 实例**（docker `UNICODE_FLAG=0`，DM8 8.1.4.6）：STARTUP 响应 @28 返回非 1/2 值，按 GB18030 编解码即可；SQL 文本、VARCHAR/CLOB 绑定与结果（含 >2048 字节的带外 CLOB）、中文表名/列名、注释、服务端中文错误消息、4 字节字符（扩展 B 区汉字、emoji）往返均正确；`lengthb('中文')=4`。测试：`crates/dm-conn/tests/gb18030.rs`（默认连 127.0.0.1:5237，连不上跳过）。
22. **TLS 证书校验**（ENABLE_ENCRYPT=1，服务端 `bin/server_ssl/` 换成自签 CA 签发的 server-cert.pem/server-key.pem，SAN=localhost,127.0.0.1）：rustls 用该 CA 校验通过（IP 与 DNS 名均可）；服务器名不符、无关 CA、仅系统根证书时握手被拒（客户端报 TLS 错误，服务端日志 `Initializing SSL Connect failed`），`ssl_verify=false` 可连。服务端不要求客户端证书。测试：`crates/dm-conn/tests/tls_verify.rs`。
