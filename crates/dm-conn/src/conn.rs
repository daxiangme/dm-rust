use crate::error::{BatchRowError, Error, Result};
use crate::options::{CompressAlgo, Compression, ConnectOptions, IsolationLevel};
use crate::param::{rewrite_jdbc_call, CallResult, OutValue, Param};
use crate::row::{ExecResult, MultiResult, QueryResult, ResultSet, Row, StatementInfo};
use crate::stream::Stream;
use bytes::{Bytes, BytesMut};
use dm_proto::codec::Codec;
use dm_proto::crypto::{Cipher, DhKeyPair, CIPHER_DES_CFB};
use dm_proto::msg::*;
use dm_proto::types::*;
use dm_proto::value::{self, ColumnCtx, LobRef};
use dm_proto::wire::{Encoding, Frame, HEADER_LEN};
use dm_proto::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, trace};

const DRIVER_VERSION: &str = "8.1.3.62";
const CLIENT_MSG_VERSION: u16 = 8;
const EC_RN_EXCEED_ROWSET_SIZE: i32 = -7036;
const EC_BP_WITH_ERROR: i32 = 121;

fn io_name(io: i16) -> &'static str {
    match io {
        0 => "IN",
        1 => "OUT",
        2 => "INOUT",
        _ => "UNKNOWN",
    }
}

/// 请求日志。LOGIN 变长区是用户名/密码（登录加密时为凭据密文），只记命令号和长度。
fn log_req(req: &[u8]) {
    let cmd = i16::from_le_bytes([req[4], req[5]]);
    let len = req.len();
    debug!(cmd, len, "req");
    if cmd != CMD_LOGIN {
        trace!("req head={:02x?}", &req[..len.min(100)]);
    }
}

const LOB_INLINE_MAX: usize = 2048;
const LOB_CHUNK: usize = 16000;

#[derive(Clone)]
struct CachedStmt {
    id: i32,
    base_name: bool,
    params: Vec<ParamDesc>,
    columns: Arc<Vec<Column>>,
    last_used: u64,
}

enum Outcome {
    Query(QueryResult),
    Exec(ExecResult),
}

pub struct Connection {
    stream: Stream,
    codec: Codec,
    enc: Encoding,
    opts: ConnectOptions,
    msg_version: u16,
    new_lob: bool,
    login: LoginResponse,
    server_version: Option<String>,
    auto_commit: bool,
    in_tx: bool,
    broken: bool,
    cache: HashMap<String, CachedStmt>,
    tick: u64,
}

fn local_tz_min() -> i16 {
    use chrono::Offset;
    (chrono::Local::now().offset().fix().local_minus_utc() / 60) as i16
}

impl Connection {
    pub async fn connect(opts: &ConnectOptions) -> Result<Connection> {
        let addr = format!("{}:{}", opts.host, opts.port);
        let tcp = tokio::time::timeout(opts.connect_timeout, TcpStream::connect(&addr))
            .await
            .map_err(|_| Error::Timeout("connect"))??;
        tcp.set_nodelay(true)?;
        let mut c = Connection {
            stream: Stream::Plain(tcp),
            codec: Codec::default(),
            enc: Encoding::Gb18030,
            opts: opts.clone(),
            msg_version: CLIENT_MSG_VERSION,
            new_lob: false,
            login: LoginResponse::default(),
            server_version: None,
            auto_commit: true,
            in_tx: false,
            broken: false,
            cache: HashMap::new(),
            tick: 0,
        };
        c.handshake().await?;
        Ok(c)
    }

    async fn handshake(&mut self) -> Result<()> {
        let o = self.opts.clone();
        // 1. STARTUP
        let dh = o.login_encrypt.then(DhKeyPair::generate);
        let (cm, cid) = (
            match o.compression {
                Compression::Off => 0,
                Compression::Always => 1,
                Compression::Auto => 2,
            },
            match o.compress_algo {
                CompressAlgo::Zlib => 0u8,
                CompressAlgo::Snappy => 1u8,
            },
        );
        let req = startup_request(&StartupParams {
            compress_mode: cm,
            compress_id: cid,
            login_encrypt: o.login_encrypt,
            bdta: false,
            msg_version: CLIENT_MSG_VERSION,
            driver_version: DRIVER_VERSION,
            dh_public: dh.as_ref().map(|d| d.public_bytes()),
        });
        let f = self.send_recv(req, true).await?;
        let sr = StartupResponse::parse(&f)?;
        debug!("startup resp: {sr:?}");
        trace!("startup raw: {:02x?}", &f.data[..]);
        self.enc = sr.encoding;
        self.msg_version = sr.msg_version.min(CLIENT_MSG_VERSION);
        self.server_version = sr.server_version.clone();
        self.codec.crc = sr.crc;
        self.codec.compress_mode = sr.compress_mode.clamp(0, 2) as u8;
        self.codec.compress_id = sr.compress_id;
        self.codec.local = o.host == "127.0.0.1" || o.host == "localhost" || o.host == "::1";
        if sr.bdta {
            return Err(Error::Other("server insists on BDTA result sets (unsupported)".into()));
        }
        // 2. TLS
        match sr.ssl_encrypt {
            0 => {}
            1 | 4 => self.upgrade_tls().await?,
            n => return Err(Error::Other(format!("unsupported ssl mode {n} (auth-only TLS needs client certificates)"))),
        }
        // 3. 加密
        let mut cred_cipher: Option<Cipher> = None;
        if let (Some(dh), Some(sp)) = (&dh, &sr.server_public) {
            let key = dh.session_key(sp);
            match sr.cipher_id {
                None | Some(-1) => cred_cipher = Some(Cipher::new(CIPHER_DES_CFB, &key)?),
                Some(id) => self.codec.cipher = Some(Cipher::new(id, &key)?),
            }
        }
        // 4. LOGIN
        let enc = self.enc;
        let (mut user, mut pass) = (enc.encode(&o.user), enc.encode(&o.password));
        if user.len() > 128 || pass.len() > 128 {
            return Err(Error::Other("user name or password longer than 128 bytes".into()));
        }
        if let Some(c) = &cred_cipher {
            user = c.encrypt(&user, false);
            pass = c.encrypt(&pass, false);
        }
        let req = login_request(&LoginParams {
            client_type: if sr.new_client_type { 5 } else { 2 },
            isolation: o.isolation.map_or(-1, |i| i.wire()),
            locale: 0,
            readonly: o.read_only,
            timezone_min: o.timezone_min.unwrap_or_else(local_tz_min),
            session_timeout: 0,
            username: &user,
            password: &pass,
            app_name: o.app_name.as_bytes(),
            os_name: std::env::consts::OS.as_bytes(),
            host_name: b"",
        });
        let f = self.send_recv(req, false).await?;
        // 登录响应不含口令；请求侧载荷已在 log_req 中省略。
        trace!("login resp raw: {:02x?}", &f.data[..]);
        self.login = LoginResponse::parse(&f, enc)?;
        self.new_lob = self.login.new_lob;
        if self.login.schema.is_empty() {
            self.login.schema = o.user.to_uppercase();
        }
        if self.server_version.is_none() {
            self.server_version = self.login.server_version.clone();
        }
        if let Some(s) = &o.schema {
            self.execute(&format!("set schema \"{}\"", s.replace('"', "\"\"")), &[]).await?;
        }
        Ok(())
    }

    #[cfg(feature = "tls")]
    async fn upgrade_tls(&mut self) -> Result<()> {
        use rustls::pki_types::ServerName;
        let tcp = match std::mem::replace(&mut self.stream, Stream::Empty) {
            Stream::Plain(t) => t,
            s => {
                self.stream = s;
                return Ok(());
            }
        };
        let name = ServerName::try_from(self.opts.host.clone()).unwrap_or_else(|_| ServerName::try_from("dameng").unwrap());
        let t = crate::stream::tls::connector().connect(name, tcp).await.map_err(|e| Error::Tls(e.to_string()))?;
        self.stream = Stream::Tls(Box::new(t));
        Ok(())
    }

    #[cfg(not(feature = "tls"))]
    async fn upgrade_tls(&mut self) -> Result<()> {
        Err(Error::Tls("server requires TLS but the `tls` feature is disabled".into()))
    }

    // ---------- 传输 ----------

    async fn send_recv(&mut self, req: BytesMut, startup: bool) -> Result<Frame> {
        if self.broken {
            return Err(Error::Broken);
        }
        // 若本 future 被取消，连接保持 broken 状态，避免错位读取。
        self.broken = true;
        let r = match self.opts.request_timeout {
            Some(t) => tokio::time::timeout(t, self.roundtrip(req, startup)).await.map_err(|_| Error::Timeout("request"))?,
            None => self.roundtrip(req, startup).await,
        };
        match &r {
            Ok(_) | Err(Error::Server { .. }) | Err(Error::Batch(_)) => self.broken = false,
            _ => {}
        }
        r
    }

    async fn roundtrip(&mut self, req: BytesMut, startup: bool) -> Result<Frame> {
        log_req(&req);
        let data = if startup { Codec::default().encode(req)? } else { self.codec.encode(req)? };
        self.stream.write_all(&data).await?;
        self.stream.flush().await?;
        let mut head = [0u8; HEADER_LEN];
        self.stream.read_exact(&mut head).await?;
        let n = Codec::body_len(&head)?;
        let mut raw = Vec::with_capacity(HEADER_LEN + n);
        raw.extend_from_slice(&head);
        raw.resize(HEADER_LEN + n, 0);
        self.stream.read_exact(&mut raw[HEADER_LEN..]).await?;
        let codec = if startup { Codec::default() } else { self.codec.clone() };
        let f = codec.decode(raw, startup)?;
        let code = f.ret_code();
        if code < 0 && code != EC_RN_EXCEED_ROWSET_SIZE {
            let e = parse_error(&f, self.enc);
            return Err(Error::Server { code: e.code, message: e.message });
        }
        Ok(f)
    }

    // ---------- 状态 ----------

    pub fn is_broken(&self) -> bool {
        self.broken
    }
    pub fn in_transaction(&self) -> bool {
        self.in_tx
    }
    pub fn server_version(&self) -> Option<&str> {
        self.server_version.as_deref()
    }
    pub fn login_info(&self) -> &LoginResponse {
        &self.login
    }
    pub fn encoding(&self) -> Encoding {
        self.enc
    }
    pub fn current_schema(&self) -> &str {
        &self.login.schema
    }

    // ---------- 语句缓存 ----------

    async fn alloc_stmt(&mut self) -> Result<(i32, bool)> {
        let f = self.send_recv(alloc_request(), false).await?;
        Ok(parse_alloc(&f)?)
    }

    async fn free_stmt(&mut self, id: i32) {
        // 释放失败不影响调用方
        let _ = self.send_recv(free_request(id), false).await;
    }

    async fn prepare_cached(&mut self, sql: &str) -> Result<CachedStmt> {
        self.tick += 1;
        if let Some(c) = self.cache.get_mut(sql) {
            c.last_used = self.tick;
            return Ok(c.clone());
        }
        let (id, base_name) = self.alloc_stmt().await?;
        let req = prepare_request(
            id,
            &self.enc.encode(sql),
            &PrepareOpts { auto_commit: self.auto_commit, direct: false, max_rows: i64::MAX, bdta: false, query_timeout: 0, msg_version: self.msg_version },
        );
        let f = match self.send_recv(req, false).await {
            Ok(f) => f,
            Err(e) => {
                self.free_stmt(id).await;
                return Err(e);
            }
        };
        let pr = PrepareResponse::parse(&f, self.enc, base_name)?;
        let st = CachedStmt { id, base_name, params: pr.params, columns: Arc::new(pr.columns), last_used: self.tick };
        if self.opts.stmt_cache_capacity > 0 {
            while self.cache.len() >= self.opts.stmt_cache_capacity {
                let victim = self.cache.iter().min_by_key(|(_, v)| v.last_used).map(|(k, _)| k.clone());
                match victim {
                    Some(k) => {
                        let v = self.cache.remove(&k).unwrap();
                        self.free_stmt(v.id).await;
                    }
                    None => break,
                }
            }
            self.cache.insert(sql.to_string(), st.clone());
        }
        Ok(st)
    }

    /// 语句未进缓存时用完即释放。
    async fn release_if_uncached(&mut self, sql: &str, st: &CachedStmt) {
        if self.opts.stmt_cache_capacity == 0 || self.cache.get(sql).map(|c| c.id) != Some(st.id) {
            self.free_stmt(st.id).await;
        }
    }

    fn forget(&mut self, sql: &str) {
        self.cache.remove(sql);
    }

    pub async fn prepare(&mut self, sql: &str) -> Result<StatementInfo> {
        let st = self.prepare_cached(sql).await?;
        let info = StatementInfo { params: st.params.clone(), columns: (*st.columns).clone() };
        self.release_if_uncached(sql, &st).await;
        Ok(info)
    }

    // ---------- 参数绑定 ----------

    fn bind(&self, st: &CachedStmt, rows: &[Vec<Value>]) -> Result<(Vec<ExecParamType>, Vec<Vec<ParamData>>, Vec<(i16, Vec<u8>)>)> {
        let n = st.params.len();
        for r in rows {
            if r.len() != n {
                return Err(Error::Other(format!("statement expects {n} parameters, got {}", r.len())));
            }
        }
        let mut types = Vec::with_capacity(n);
        for (i, sp) in st.params.iter().enumerate() {
            let first = rows.iter().map(|r| &r[i]).find(|v| !v.is_null());
            let (mut ty, mut prec, mut scale) = match first {
                None => (sp.col_type, sp.prec, sp.scale),
                Some(v) => {
                    if sp.col_type != UNKNOWN && !sp.type_recommended() {
                        (sp.col_type, sp.prec, sp.scale)
                    } else {
                        v.natural_type()
                    }
                }
            };
            if ty == UNKNOWN || ty == NULL {
                (ty, prec, scale) = (VARCHAR, VARCHAR_PREC, 0);
            }
            if is_char(ty) || matches!(ty, BINARY | VARBINARY) {
                let maxlen = rows
                    .iter()
                    .filter_map(|r| match &r[i] {
                        Value::String(s) => Some(self.enc.encode(s).len() as i32),
                        Value::Bytes(b) => Some(b.len() as i32),
                        Value::Null => None,
                        other => other.to_text().map(|t| t.len() as i32),
                    })
                    .max()
                    .unwrap_or(0);
                if maxlen > prec {
                    prec = value::bucket_prec(maxlen);
                }
            }
            types.push(ExecParamType { io_type: 0, col_type: ty, prec, scale });
        }
        let mut out = Vec::with_capacity(rows.len());
        let mut uploads = Vec::new();
        for r in rows {
            let mut row = Vec::with_capacity(n);
            for (i, v) in r.iter().enumerate() {
                if v.is_null() {
                    row.push(ParamData::Null);
                } else {
                    // INTERVAL 值内 scale 与参数描述的 scale 不一致时，服务端返回 -6117。
                    let b = value::encode_value_with_scale(v, types[i].col_type, Some(types[i].scale), self.enc)?;
                    if matches!(types[i].col_type, BLOB | CLOB) && b.len() > LOB_INLINE_MAX {
                        if rows.len() != 1 {
                            return Err(Error::Other("批量执行不支持大于 2048 字节的 LOB 参数".into()));
                        }
                        uploads.push((i as i16, b));
                        row.push(ParamData::OffRow);
                    } else {
                        row.push(ParamData::Inline(b));
                    }
                }
            }
            out.push(row);
        }
        Ok((types, out, uploads))
    }

    // ---------- 执行 ----------

    async fn run(&mut self, sql: &str, rows: &[Vec<Value>]) -> Result<Outcome> {
        let mut retried = false;
        loop {
            let r = self.run_once(sql, rows).await;
            // -7009 对象定义被修改：缓存的语句计划已过期（表被 DDL 修改），重新 prepare 重试一次
            if !retried && matches!(&r, Err(Error::Server { code: -7009, .. })) {
                retried = true;
                continue;
            }
            return r;
        }
    }

    async fn run_once(&mut self, sql: &str, rows: &[Vec<Value>]) -> Result<Outcome> {
        let st = self.prepare_cached(sql).await?;
        let r = self.run_stmt(sql, &st, rows).await;
        if let Err(e) = &r {
            if matches!(e, Error::Server { .. }) {
                // 服务端报错后缓存的语句可能已失效（如 DDL 导致重编译），保守起见丢弃
                if self.cache.get(sql).map(|c| c.id) == Some(st.id) {
                    self.forget(sql);
                }
                self.free_stmt(st.id).await;
                return r;
            }
        }
        self.release_if_uncached(sql, &st).await;
        r
    }

    async fn run_stmt(&mut self, sql: &str, st: &CachedStmt, rows: &[Vec<Value>]) -> Result<Outcome> {
        let f = self.send_exec(st, rows).await?;
        let er = ExecResponse::parse(&f, self.enc, st.base_name, &st.columns)?;
        self.outcome_of(sql, st, er, true).await
    }

    /// 有列或有行才是结果集。sql type 162 且列、行都空时是更新计数（多语句 DML、匿名块），不是空查询。
    fn is_row_result(er: &ExecResponse) -> bool {
        is_select(er.sql_type) && (!er.columns.is_empty() || !er.rows.is_empty())
    }

    async fn send_exec(&mut self, st: &CachedStmt, rows: &[Vec<Value>]) -> Result<Frame> {
        if st.params.iter().any(|p| p.io_type == 1 || p.io_type == 2 || p.col_type == CURSOR) {
            return Err(Error::Other("statement has OUT, INOUT, or REF CURSOR parameters; use Connection::call".into()));
        }
        let (types, data, uploads) = if st.params.is_empty() { (vec![], vec![], vec![]) } else { self.bind(st, rows)? };
        if !uploads.is_empty() {
            let f = self.send_recv(pre_exec_request(st.id, &types), false).await?;
            drop(f);
            for (idx, bytes) in &uploads {
                for chunk in bytes.chunks(LOB_CHUNK) {
                    self.send_recv(put_data_request(st.id, *idx, chunk, self.new_lob), false).await?;
                }
            }
        }
        let req = exec_request(
            st.id,
            &ExecOpts { auto_commit: self.auto_commit, msg_version: self.msg_version, ..Default::default() },
            &types,
            &data,
        )?;
        let f = self.send_recv(req, false).await?;
        if f.ret_code() == EC_BP_WITH_ERROR {
            let errs = parse_batch_errors(&f, self.enc)?;
            return Err(Error::Batch(errs.into_iter().map(|e| BatchRowError { row: e.row, code: e.code, message: e.message }).collect()));
        }
        trace!("exec raw: {:02x?}", &f.data[..]);
        Ok(f)
    }

    async fn outcome_of(&mut self, sql: &str, st: &CachedStmt, er: ExecResponse, update_cache: bool) -> Result<Outcome> {
        self.update_tx(&er);
        if Self::is_row_result(&er) {
            let cols = Arc::new(er.columns.clone());
            if update_cache && er.columns_from_response {
                // 服务端在计划重编译/首次执行时会在 EXECUTE 响应里给出最终列类型，更新缓存。
                // 多结果集只缓存第一份；后续结果各自带列描述。
                if let Some(c) = self.cache.get_mut(sql) {
                    if c.id == st.id {
                        c.columns = cols.clone();
                    }
                }
            }
            let rows = self.drain_rows(st.id, &cols, er.rows, er.update_count, er.rows_in_packet, 0).await?;
            Ok(Outcome::Query(QueryResult { columns: cols, rows }))
        } else {
            Ok(Outcome::Exec(ExecResult {
                rows_affected: if er.update_count == i64::MAX { -1 } else { er.update_count },
                batch_counts: er.batch_counts,
                insert_ids: er.insert_ids,
                print_msg: er.print_msg,
            }))
        }
    }

    /// 从 EXECUTE 响应起，用 NEXT_RESULT(@20=0) 取完后续结果，直到返回码 +111 或 sql type 0。
    async fn collect_results(&mut self, sql: &str, st: &CachedStmt, rows: &[Vec<Value>]) -> Result<MultiResult> {
        let mut frame = self.send_exec(st, rows).await?;
        let mut results = Vec::new();
        let mut print_msg = String::new();
        let mut update_cache = true;
        loop {
            if no_more_results(frame.ret_code(), frame.i16_at(20).unwrap_or(0)) {
                break;
            }
            let er = ExecResponse::parse(&frame, self.enc, st.base_name, &st.columns)?;
            if !er.print_msg.is_empty() {
                if !print_msg.is_empty() {
                    print_msg.push('\n');
                }
                print_msg.push_str(&er.print_msg);
            }
            let outcome = self.outcome_of(sql, st, er, update_cache).await?;
            update_cache = false;
            results.push(match outcome {
                Outcome::Query(q) => ResultSet::Query(q),
                Outcome::Exec(e) => ResultSet::Update(e),
            });
            frame = self.send_recv(next_result_request(st.id, 0), false).await?;
        }
        Ok(MultiResult { results, print_msg })
    }

    /// 把已经收到的行补全到结果集结束。`rs_id` 为 0 表示当前结果集（查询，以及 NEXT_RESULT 之后的 REF CURSOR）。
    async fn drain_rows(&mut self, stmt_id: i32, cols: &Arc<Vec<Column>>, mut raw: Vec<RawRow>, mut total: i64, rows_in_packet: u32, rs_id: i16) -> Result<Vec<Row>> {
        let mut got = raw.len() as i64;
        if (total == i64::MAX || got < total) && !raw.is_empty() || (got < total && rows_in_packet == 0) {
            loop {
                if total != i64::MAX && got >= total {
                    break;
                }
                let req = fetch_request(stmt_id, got, self.opts.fetch_size, rs_id, 0);
                debug!(got, total, rows_in_packet, "fetch");
                trace!("fetch req: {:02x?}", &req[..]);
                let f = match self.send_recv(req, false).await {
                    Ok(f) => f,
                    // 游标耗尽后再取会返回 -2002（执行未准备SQL语句），视为结束
                    Err(Error::Server { code: -2002, .. }) => break,
                    Err(e) => return Err(e),
                };
                let fr = FetchResponse::parse(&f, cols.len())?;
                total = fr.total;
                if fr.rows.is_empty() {
                    break;
                }
                got += fr.rows.len() as i64;
                raw.extend(fr.rows);
            }
        }
        let mut out = Vec::with_capacity(raw.len());
        for r in raw {
            out.push(self.decode_row(stmt_id, cols, r).await?);
        }
        Ok(out)
    }

    fn update_tx(&mut self, er: &ExecResponse) {
        if self.auto_commit {
            self.in_tx = false;
        } else {
            self.in_tx = true;
        }
        let _ = er.trx_status;
    }

    async fn decode_row(&mut self, stmt_id: i32, cols: &Arc<Vec<Column>>, raw: RawRow) -> Result<Row> {
        let mut values = Vec::with_capacity(raw.len());
        for (c, d) in cols.iter().zip(raw) {
            let v = match d {
                None => Value::Null,
                Some(b) => {
                    let ctx = ColumnCtx { col_type: c.col_type, new_lob: self.new_lob, lob_tab_id: c.lob_tab_id, lob_col_id: c.lob_col_id };
                    match value::decode_value(&ctx, b, self.enc)? {
                        Value::Lob(l) => self.resolve_lob(stmt_id, l).await?,
                        v => v,
                    }
                }
            };
            values.push(v);
        }
        Ok(Row { columns: cols.clone(), values })
    }

    async fn resolve_lob(&mut self, stmt_id: i32, l: LobRef) -> Result<Value> {
        let data: Bytes = if let Some(b) = l.inline {
            b
        } else {
            let chunk = if l.is_clob { LOB_READ_CHUNK_CLOB } else { LOB_READ_CHUNK_BLOB };
            let mut buf = BytesMut::new();
            let mut cur = LobCursor { cur_file_id: l.head.file_id, cur_page_no: l.head.page_no, total_offset: 0 };
            let mut offset: i32 = 0;
            loop {
                let req = lob_read_request(stmt_id, l.is_clob, &l.head, &cur, offset, chunk, self.new_lob);
                let f = self.send_recv(req, false).await?;
                let r = parse_lob_read(&f)?;
                if r.data.is_empty() {
                    break;
                }
                offset += if l.is_clob { r.char_len.map(|c| c as i32).unwrap_or_else(|| self.enc.decode(&r.data).chars().count() as i32) } else { r.data.len() as i32 };
                cur = r.cursor;
                buf.extend_from_slice(&r.data);
                if r.read_over {
                    break;
                }
            }
            buf.freeze()
        };
        Ok(if l.is_clob { Value::String(self.enc.decode(&data)) } else { Value::Bytes(data) })
    }

    /// 执行查询并取回全部行。
    pub async fn query(&mut self, sql: &str, params: &[Value]) -> Result<QueryResult> {
        match self.run(sql, &[params.to_vec()]).await? {
            Outcome::Query(q) => Ok(q),
            Outcome::Exec(_) => Ok(QueryResult::default()),
        }
    }

    /// 执行 DML/DDL，返回影响行数。
    pub async fn execute(&mut self, sql: &str, params: &[Value]) -> Result<ExecResult> {
        match self.run(sql, &[params.to_vec()]).await? {
            Outcome::Exec(e) => Ok(e),
            Outcome::Query(q) => Ok(ExecResult { rows_affected: q.rows.len() as i64, ..Default::default() }),
        }
    }

    /// 批量执行：一条消息携带多行参数。
    pub async fn execute_batch(&mut self, sql: &str, rows: &[Vec<Value>]) -> Result<ExecResult> {
        if rows.is_empty() {
            return Ok(ExecResult::default());
        }
        match self.run(sql, rows).await? {
            Outcome::Exec(e) => Ok(e),
            Outcome::Query(q) => Ok(ExecResult { rows_affected: q.rows.len() as i64, ..Default::default() }),
        }
    }

    /// 取回一次执行产生的全部结果集。
    ///
    /// 多个 `SELECT` 用分号写在同一条 SQL 里时，每个查询是一项 [`ResultSet::Query`]（包括 0 行）。
    /// 本服务器上，夹在中间的 DML 会执行，但不会单独占一项；纯 DML 批只有一项 [`ResultSet::Update`]，
    /// 影响行数是响应里的那一个计数（实测为最后一条语句的行数，不是逐条列表）。
    /// 批量参数的逐行计数仍在 [`ExecResult::batch_counts`] 里，不走 NEXT_RESULT。
    /// [`Connection::query`] / [`Connection::execute`] 只解释第一份响应，不向后取。
    /// OUT、INOUT 与 `SYS_REFCURSOR` 仍用 [`Connection::call`]。
    pub async fn query_multi(&mut self, sql: &str, params: &[Value]) -> Result<MultiResult> {
        let mut retried = false;
        loop {
            let r = self.query_multi_once(sql, &[params.to_vec()]).await;
            if !retried && matches!(&r, Err(Error::Server { code: -7009, .. })) {
                retried = true;
                continue;
            }
            return r;
        }
    }

    async fn query_multi_once(&mut self, sql: &str, rows: &[Vec<Value>]) -> Result<MultiResult> {
        let st = self.prepare_cached(sql).await?;
        let r = self.collect_results(sql, &st, rows).await;
        if let Err(e) = &r {
            if matches!(e, Error::Server { .. }) {
                if self.cache.get(sql).map(|c| c.id) == Some(st.id) {
                    self.forget(sql);
                }
                self.free_stmt(st.id).await;
                return r;
            }
        }
        self.release_if_uncached(sql, &st).await;
        r
    }

    pub async fn query_one(&mut self, sql: &str, params: &[Value]) -> Result<Option<Row>> {
        Ok(self.query(sql, params).await?.rows.into_iter().next())
    }

    pub async fn ping(&mut self) -> Result<()> {
        self.query("select 1", &[]).await.map(|_| ())
    }

    // ---------- 事务 ----------

    pub fn auto_commit(&self) -> bool {
        self.auto_commit
    }

    /// 切换自动提交（客户端状态，随每个请求发送）。
    pub fn set_auto_commit(&mut self, on: bool) {
        self.auto_commit = on;
        if on {
            self.in_tx = false;
        }
    }

    pub async fn begin(&mut self) -> Result<()> {
        self.auto_commit = false;
        self.in_tx = true;
        Ok(())
    }

    pub async fn commit(&mut self) -> Result<()> {
        self.send_recv(commit_request(), false).await?;
        self.auto_commit = true;
        self.in_tx = false;
        Ok(())
    }

    pub async fn rollback(&mut self) -> Result<()> {
        self.send_recv(rollback_request(), false).await?;
        self.auto_commit = true;
        self.in_tx = false;
        Ok(())
    }

    pub async fn set_isolation(&mut self, level: IsolationLevel) -> Result<()> {
        self.send_recv(set_isolation_request(level.wire()), false).await?;
        Ok(())
    }

    /// 调用存储过程、存储函数或匿名 PL/SQL 块，返回 OUT 与 INOUT 的值。
    ///
    /// `sql` 可以是 `begin p(?, ?, ?); end;`、`declare ... begin ... end;`、`call p(?, ?)`，
    /// 也可以是 JDBC 转义 `{call p(?, ?)}` 或 `{?=call f(?, ?)}`。花括号形式由客户端改写成
    /// 匿名块后再发送（服务端不接受该转义，会报 -2007）。
    ///
    /// 参数个数必须与语句一致。PREPARE 给出的 ioType 为 0/1/2 时必须与 [`Param`] 的方向相同，
    /// 不一致则直接返回错误，不会发送 EXECUTE。ioType 为 -1 时采用调用方声明的方向。
    /// 服务端已经给出具体类型时按该类型编解码（即使 itemFlag 标了“仅推荐”）；类型为 UNKNOWN 时才用值的自然类型或 `TypeHint`。
    ///
    /// `outs` 只包含 OUT 与 INOUT，顺序与参数列表一致。`SYS_REFCURSOR` 为 [`OutValue::Cursor`]，行已全部取回。
    /// 纯 IN 语句仍用 [`Connection::query`] / [`Connection::execute`]。
    pub async fn call(&mut self, sql: &str, params: &[Param]) -> Result<CallResult> {
        let sql = rewrite_jdbc_call(sql);
        let mut retried = false;
        loop {
            let r = self.call_once(&sql, params).await;
            // 与 query/execute 相同：-7009 表示缓存的语句计划已过期，重新 prepare 再试一次。
            if !retried && matches!(&r, Err(Error::Server { code: -7009, .. })) {
                retried = true;
                continue;
            }
            return r;
        }
    }

    async fn call_once(&mut self, sql: &str, params: &[Param]) -> Result<CallResult> {
        let st = self.prepare_cached(sql).await?;
        let r = self.call_stmt(&st, params).await;
        if let Err(e) = &r {
            if matches!(e, Error::Server { .. }) {
                if self.cache.get(sql).map(|c| c.id) == Some(st.id) {
                    self.forget(sql);
                }
                self.free_stmt(st.id).await;
                return r;
            }
        }
        self.release_if_uncached(sql, &st).await;
        r
    }

    fn bind_call(&self, st: &CachedStmt, params: &[Param]) -> Result<(Vec<ExecParamType>, Vec<ParamData>, Vec<(i16, Vec<u8>)>)> {
        if params.len() != st.params.len() {
            return Err(Error::Other(format!("statement expects {} parameters, got {}", st.params.len(), params.len())));
        }
        let mut types = Vec::with_capacity(params.len());
        let mut data = Vec::with_capacity(params.len());
        let mut uploads = Vec::new();
        for (i, (sp, p)) in st.params.iter().zip(params.iter()).enumerate() {
            let user_io = p.io_type();
            let io = if matches!(sp.io_type, 0 | 1 | 2) {
                if sp.io_type != user_io {
                    return Err(Error::Other(format!(
                        "parameter {} direction mismatch: server ioType is {}, argument is {}",
                        i + 1,
                        io_name(sp.io_type),
                        io_name(user_io)
                    )));
                }
                sp.io_type
            } else {
                user_io
            };

            let (ty, mut prec, scale) = if sp.col_type != UNKNOWN && sp.col_type != NULL {
                (sp.col_type, sp.prec, sp.scale)
            } else {
                match p {
                    Param::In(v) | Param::InOut(v) if !v.is_null() => v.natural_type(),
                    Param::In(_) | Param::InOut(_) => (VARCHAR, VARCHAR_PREC, 0),
                    Param::Out(h) => {
                        if h.col_type == UNKNOWN || h.col_type == NULL {
                            (VARCHAR, VARCHAR_PREC, 0)
                        } else {
                            (h.col_type, h.prec, h.scale)
                        }
                    }
                }
            };
            if matches!(p, Param::In(v) | Param::InOut(v) if !v.is_null()) && (is_char(ty) || matches!(ty, BINARY | VARBINARY)) {
                let v = match p {
                    Param::In(v) | Param::InOut(v) => v,
                    Param::Out(_) => unreachable!(),
                };
                let len = match v {
                    Value::String(s) => self.enc.encode(s).len() as i32,
                    Value::Bytes(b) => b.len() as i32,
                    other => other.to_text().map(|t| t.len() as i32).unwrap_or(0),
                };
                if len > prec {
                    prec = value::bucket_prec(len);
                }
            }
            types.push(ExecParamType { io_type: io as i8, col_type: ty, prec, scale });

            // OUT 不写值。CURSOR 也不写（实测写 i32 长度 + stmt id 会被服务端拒绝）。
            if io == 1 || ty == CURSOR {
                data.push(ParamData::Absent);
                continue;
            }
            let v = match p {
                Param::In(v) | Param::InOut(v) => v,
                Param::Out(_) => return Err(Error::Other(format!("parameter {} is OUT on the client but IN on the server", i + 1))),
            };
            if v.is_null() {
                data.push(ParamData::Null);
            } else {
                let b = value::encode_value_with_scale(v, ty, Some(scale), self.enc)?;
                if matches!(ty, BLOB | CLOB) && b.len() > LOB_INLINE_MAX {
                    uploads.push((i as i16, b));
                    data.push(ParamData::OffRow);
                } else {
                    data.push(ParamData::Inline(b));
                }
            }
        }
        Ok((types, data, uploads))
    }

    async fn call_stmt(&mut self, st: &CachedStmt, params: &[Param]) -> Result<CallResult> {
        let (types, data, uploads) = self.bind_call(st, params)?;
        if !uploads.is_empty() {
            self.send_recv(pre_exec_request(st.id, &types), false).await?;
            for (idx, bytes) in &uploads {
                for chunk in bytes.chunks(LOB_CHUNK) {
                    self.send_recv(put_data_request(st.id, *idx, chunk, self.new_lob), false).await?;
                }
            }
        }
        let req = exec_request(
            st.id,
            &ExecOpts { auto_commit: self.auto_commit, msg_version: self.msg_version, ..Default::default() },
            &types,
            &[data],
        )?;
        let f = self.send_recv(req, false).await?;
        trace!("exec raw: {:02x?}", &f.data[..]);
        // 过程调用的列来自本次响应里的游标，不用 PREPARE 的列，也不写回语句缓存。
        let mut er = ExecResponse::parse(&f, self.enc, st.base_name, &[])?;
        self.update_tx(&er);

        let mut outs = Vec::new();
        let mut out_slot = 0usize;
        let mut cursor_ord = 0usize;
        for (i, p) in params.iter().enumerate() {
            if !matches!(p, Param::Out(_) | Param::InOut(_)) {
                continue;
            }
            if out_slot >= er.out_params.len() {
                return Err(Error::Other(format!(
                    "server returned {} OUT values, parameter {} still expects one",
                    er.out_params.len(),
                    i + 1
                )));
            }
            let ty = types[i].col_type;
            if ty == CURSOR {
                let q = if cursor_ord == 0 {
                    cursor_ord += 1;
                    let raw = std::mem::take(&mut er.rows);
                    self.cursor_result(st, &er, raw).await?
                } else {
                    cursor_ord += 1;
                    let f = self.send_recv(next_result_request(st.id, 0), false).await?;
                    let mut nr = ExecResponse::parse(&f, self.enc, st.base_name, &[])?;
                    let raw = std::mem::take(&mut nr.rows);
                    self.cursor_result(st, &nr, raw).await?
                };
                outs.push(OutValue::Cursor(q));
            } else {
                let raw = er.out_params[out_slot].clone();
                let v = match raw {
                    None => Value::Null,
                    Some(b) => {
                        let sp = &st.params[i];
                        let ctx = ColumnCtx { col_type: ty, new_lob: self.new_lob, lob_tab_id: sp.lob_tab_id, lob_col_id: sp.lob_col_id };
                        match value::decode_value(&ctx, b, self.enc)? {
                            Value::Lob(l) => self.resolve_lob(st.id, l).await?,
                            v => v,
                        }
                    }
                };
                outs.push(OutValue::Value(v));
            }
            out_slot += 1;
        }
        if out_slot != er.out_params.len() {
            return Err(Error::Other(format!("server returned {} OUT values, {} were consumed", er.out_params.len(), out_slot)));
        }
        Ok(CallResult {
            outs,
            rows_affected: if er.update_count == i64::MAX { -1 } else { er.update_count },
            print_msg: er.print_msg,
        })
    }

    async fn cursor_result(&mut self, st: &CachedStmt, er: &ExecResponse, raw: Vec<RawRow>) -> Result<QueryResult> {
        if !(is_select(er.sql_type) && (er.columns_from_response || !raw.is_empty())) {
            return Err(Error::Other(format!(
                "expected REF CURSOR result set, got sql_type={} columns={} rows={} update_count={}",
                er.sql_type,
                er.columns.len(),
                raw.len(),
                er.update_count
            )));
        }
        let cols = Arc::new(er.columns.clone());
        let rows = self.drain_rows(st.id, &cols, raw, er.update_count, er.rows_in_packet, 0).await?;
        Ok(QueryResult { columns: cols, rows })
    }

    /// 释放全部语句句柄并关闭连接。
    pub async fn close(mut self) -> Result<()> {
        let ids: Vec<i32> = self.cache.values().map(|c| c.id).collect();
        for id in ids {
            self.free_stmt(id).await;
        }
        let _ = self.stream.shutdown().await;
        Ok(())
    }
}

