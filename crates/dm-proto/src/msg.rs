//! 各命令的请求构造与响应解析（规格 §3–§12）。
use crate::error::{ProtoError, Result};
use crate::types::*;
use crate::value::LobHead;
use crate::wire::{Encoding, Frame, Reader, Writer};
use bytes::{Bytes, BytesMut};

pub const CMD_LOGIN: i16 = 1;
pub const CMD_STMT_ALLOC: i16 = 3;
pub const CMD_STMT_FREE: i16 = 4;
pub const CMD_PREPARE: i16 = 5;
pub const CMD_FETCH: i16 = 7;
pub const CMD_COMMIT: i16 = 8;
pub const CMD_ROLLBACK: i16 = 9;
pub const CMD_EXECUTE: i16 = 13;
pub const CMD_PUT_DATA: i16 = 14;
pub const CMD_LOB_LENGTH: i16 = 29;
pub const CMD_LOB_READ: i16 = 32;
pub const CMD_NEXT_RESULT: i16 = 44;
pub const CMD_SET_ISOLATION: i16 = 52;
pub const CMD_PRE_EXEC: i16 = 90;
pub const CMD_STARTUP: i16 = 200;

/// 单次 LOB 读取的上限（BLOB 字节 / CLOB 字符）。
pub const LOB_READ_CHUNK_BLOB: i32 = 32000;
pub const LOB_READ_CHUNK_CLOB: i32 = 16000;
/// 超过该字节数的 LOB 参数走带外上传。
pub const LOB_INLINE_MAX: usize = 2048;
pub const VALUE_MAX_INLINE: usize = 0xFF00;

pub const NULL_MARK: u16 = 0xFFFE;
pub const LONG_MARK: u16 = 0xFFFF;

// ---------- STARTUP ----------

pub struct StartupParams<'a> {
    pub compress_mode: i32,
    pub compress_id: u8,
    pub login_encrypt: bool,
    pub bdta: bool,
    pub msg_version: u16,
    pub driver_version: &'a str,
    pub dh_public: Option<[u8; 64]>,
}

pub fn startup_request(p: &StartupParams) -> BytesMut {
    let mut w = Writer::new(CMD_STARTUP, 0);
    w.set_i32(20, 0);
    w.set_i32(24, p.compress_mode);
    w.set_u8(28, p.login_encrypt as u8);
    w.set_u8(29, if p.login_encrypt { 2 } else { 0 });
    w.set_u8(30, if p.bdta { 2 } else { 0 });
    w.set_u8(31, p.compress_id);
    w.set_u8(32, 0);
    w.set_u8(33, 0);
    w.set_u8(34, 1);
    w.set_u16(35, p.msg_version);
    w.push_i32_bytes(p.driver_version.as_bytes());
    w.push_u8(0);
    if p.login_encrypt {
        let k = p.dh_public.expect("dh public key required when login_encrypt");
        w.push_i32_bytes(&k);
    }
    w.finish()
}

#[derive(Debug, Clone)]
pub struct StartupResponse {
    pub ssl_encrypt: i32,
    pub server_series: i32,
    pub encoding: Encoding,
    pub compress_mode: i32,
    pub bdta: bool,
    pub compress_id: u8,
    pub crc: bool,
    pub new_client_type: bool,
    pub msg_version: u16,
    pub server_version: Option<String>,
    pub cipher_id: Option<i32>,
    pub server_public: Option<Bytes>,
    pub hash_id: Option<i32>,
}

impl StartupResponse {
    pub fn parse(f: &Frame) -> Result<Self> {
        let key_flag = f.u8_at(40)?;
        let cipher_flag = f.u8_at(41)?;
        let msg_version = f.u16_at(48)?;
        // 实测：STARTUP 响应的变长区从 @80 开始（@64..@79 为保留字段）
        let mut r = Reader::new(f.data.clone(), 80);
        let server_version = if msg_version < 21 { Some(r.i32_string(Encoding::Utf8)?) } else { None };
        let cipher_id = if cipher_flag > 0 { Some(r.i32()?) } else { None };
        let server_public = if key_flag > 0 { Some(r.i32_bytes()?) } else { None };
        let hash_id = if cipher_flag == 2 { Some(r.i32()?) } else { None };
        Ok(StartupResponse {
            ssl_encrypt: f.i32_at(20)?,
            server_series: f.i32_at(24)?,
            encoding: Encoding::from_code(f.i32_at(28)?),
            compress_mode: f.i32_at(32)?,
            bdta: f.u8_at(42)? > 0,
            compress_id: f.u8_at(43)?,
            crc: f.u8_at(45)? == 1,
            new_client_type: f.u8_at(47)? == 1,
            msg_version,
            server_version,
            cipher_id,
            server_public,
            hash_id,
        })
    }
}

// ---------- LOGIN ----------

pub struct LoginParams<'a> {
    pub client_type: i32,
    pub isolation: i32,
    pub locale: i32,
    pub readonly: bool,
    pub timezone_min: i16,
    pub session_timeout: i32,
    /// 已按需加密的用户名/密码字节
    pub username: &'a [u8],
    pub password: &'a [u8],
    pub app_name: &'a [u8],
    pub os_name: &'a [u8],
    pub host_name: &'a [u8],
}

pub fn login_request(p: &LoginParams) -> BytesMut {
    let mut w = Writer::new(CMD_LOGIN, 0);
    w.set_i32(20, p.client_type);
    w.set_i32(24, p.isolation);
    w.set_i32(28, p.locale);
    w.set_u8(32, p.readonly as u8);
    w.set_i16(33, p.timezone_min);
    w.set_i32(35, p.session_timeout);
    w.set_u8(39, 0);
    w.set_u8(40, 0);
    w.set_u8(41, 1);
    w.set_u8(42, 0);
    w.push_i32_bytes(p.username);
    w.push_i32_bytes(p.password);
    w.push_i32_bytes(p.app_name);
    w.push_i32_bytes(p.os_name);
    w.push_i32_bytes(p.host_name);
    w.push_u8(0);
    w.push_u16(0);
    w.finish()
}

#[derive(Debug, Clone, Default)]
pub struct LoginResponse {
    pub max_row_size: i32,
    pub ddl_auto_commit: bool,
    pub isolation: i32,
    pub case_sensitive: bool,
    pub backslash_escape: bool,
    pub server_mode: u16,
    pub server_status: u16,
    pub db_timezone_min: i16,
    pub new_lob: bool,
    pub buf_prefetch: i32,
    pub instance_name: String,
    pub schema: String,
    pub db_name: String,
    pub session_id: Option<i64>,
    pub oracle_format: bool,
    pub date_formats: Option<[String; 5]>,
    pub server_version: Option<String>,
}

impl LoginResponse {
    pub fn parse(f: &Frame, enc: Encoding) -> Result<Self> {
        let standby = f.u8_at(42)? == 1;
        let ver_len = f.i32_at(55).unwrap_or(0);
        let mut out = LoginResponse {
            max_row_size: f.i32_at(20)?,
            ddl_auto_commit: f.u8_at(28)? != 0,
            isolation: f.i32_at(29)?,
            case_sensitive: f.u8_at(33)? != 0,
            backslash_escape: f.u8_at(34)? != 0,
            server_mode: f.u16_at(35)?,
            server_status: f.u16_at(37)?,
            db_timezone_min: f.i16_at(40)?,
            new_lob: f.u8_at(43)? == 1,
            buf_prefetch: f.i32_at(44)?,
            ..Default::default()
        };
        // 实测：LOGIN 响应的变长区从 @80 开始
        let mut r = Reader::new(f.data.clone(), 80);
        out.instance_name = r.i32_string(enc)?;
        out.schema = r.i32_string(enc)?;
        let _last_ip = r.i32_bytes()?;
        let _last_time = r.i32_bytes()?;
        r.skip(12)?; // 失败次数、警告 id、宽限期
        let _guid = r.i32_bytes()?;
        out.db_name = r.i32_string(enc)?;
        if standby {
            let _host = r.i32_bytes()?;
            r.skip(4)?;
            r.skip(2)?;
        }
        if r.remaining() >= 8 {
            out.session_id = Some(r.i64()?);
        }
        if r.remaining() >= 1 {
            out.oracle_format = r.u8()? == 1;
        }
        if r.remaining() >= 5 {
            let mut v: [String; 5] = Default::default();
            for s in v.iter_mut() {
                *s = r.u8_string(enc)?;
            }
            out.date_formats = Some(v);
        }
        if r.remaining() > 0 && ver_len > 0 {
            out.server_version = Some(r.string_n((ver_len as usize).min(r.remaining()), enc)?);
        }
        Ok(out)
    }
}

// ---------- 语句 ----------

pub fn alloc_request() -> BytesMut {
    let mut w = Writer::new(CMD_STMT_ALLOC, 0);
    w.set_u8(20, 1);
    w.finish()
}

/// 返回 (stmt_id, 列描述是否带 base name)
pub fn parse_alloc(f: &Frame) -> Result<(i32, bool)> {
    Ok((f.stmt_id(), f.u8_at(20).unwrap_or(0) == 1))
}

pub fn free_request(stmt_id: i32) -> BytesMut {
    Writer::new(CMD_STMT_FREE, stmt_id).finish()
}

pub fn commit_request() -> BytesMut {
    Writer::new(CMD_COMMIT, 0).finish()
}

pub fn rollback_request() -> BytesMut {
    Writer::new(CMD_ROLLBACK, 0).finish()
}

pub fn set_isolation_request(level: i32) -> BytesMut {
    let mut w = Writer::new(CMD_SET_ISOLATION, 0);
    w.set_i32(20, level);
    w.finish()
}

pub struct PrepareOpts {
    pub auto_commit: bool,
    /// true = 直接执行（命令 5 的 @21 = 1）
    pub direct: bool,
    pub max_rows: i64,
    pub bdta: bool,
    pub query_timeout: i32,
    pub msg_version: u16,
}

pub fn prepare_request(stmt_id: i32, sql: &[u8], o: &PrepareOpts) -> BytesMut {
    let mut w = Writer::new(CMD_PREPARE, stmt_id);
    w.set_u8(20, o.auto_commit as u8);
    w.set_u8(21, o.direct as u8);
    w.set_u8(22, 0);
    w.set_u8(23, 1);
    w.set_u8(24, 0);
    w.set_i16(25, 0);
    w.set_i64(27, o.max_rows);
    w.set_u8(35, if o.bdta { 2 } else { 0 });
    w.set_i16(36, 0);
    if o.msg_version >= 3 {
        w.set_u8(38, o.direct as u8);
    }
    w.set_u8(39, 0);
    w.set_u8(40, 0);
    w.set_i32(41, o.query_timeout);
    w.set_u8(45, 0);
    w.push_cstr(sql);
    w.finish()
}

/// 参数描述（PREPARE 响应）。
#[derive(Debug, Clone, Default)]
pub struct ParamDesc {
    pub col_type: i32,
    pub prec: i32,
    pub scale: i32,
    pub nullable: i32,
    pub item_flag: i16,
    pub io_type: i16,
    pub name: String,
    pub type_name: String,
    pub table_name: String,
    pub schema_name: String,
    pub lob_tab_id: i32,
    pub lob_col_id: i16,
}

impl ParamDesc {
    pub fn is_lob_flag(&self) -> bool {
        self.item_flag & 2 != 0
    }
    /// 服务端类型仅为推荐（bit 8）
    pub fn type_recommended(&self) -> bool {
        self.item_flag & 8 != 0
    }
}

/// 列描述（PREPARE / EXEC 响应）。
#[derive(Debug, Clone, Default)]
pub struct Column {
    pub col_type: i32,
    pub prec: i32,
    pub scale: i32,
    pub nullable: i32,
    pub item_flag: i16,
    pub name: String,
    pub type_name: String,
    pub table_name: String,
    pub schema_name: String,
    pub base_name: Option<String>,
    pub lob_tab_id: i32,
    pub lob_col_id: i16,
    pub oracle_date: bool,
    pub local_tz: bool,
    pub oracle_float: bool,
    pub bfile: bool,
}

impl Column {
    pub fn is_lob(&self) -> bool {
        self.item_flag & 2 != 0 || is_lob(self.col_type)
    }
}

fn parse_param(r: &mut Reader, enc: Encoding) -> Result<ParamDesc> {
    let mut p = ParamDesc { col_type: r.i32()?, prec: r.i32()?, scale: r.i32()?, nullable: r.i32()?, item_flag: r.i16()?, ..Default::default() };
    r.skip(4)?;
    p.io_type = r.i16()?;
    let l = [r.i16()?, r.i16()?, r.i16()?, r.i16()?];
    p.name = r.string_n(l[0].max(0) as usize, enc)?;
    p.type_name = r.string_n(l[1].max(0) as usize, enc)?;
    p.table_name = r.string_n(l[2].max(0) as usize, enc)?;
    p.schema_name = r.string_n(l[3].max(0) as usize, enc)?;
    if p.item_flag & 2 != 0 {
        p.lob_tab_id = r.i32()?;
        p.lob_col_id = r.i16()?;
    }
    Ok(p)
}

pub fn parse_column(r: &mut Reader, enc: Encoding, base_name: bool) -> Result<Column> {
    let mut c = Column { col_type: r.i32()?, prec: r.i32()?, scale: r.i32()?, nullable: r.i32()?, item_flag: r.i16()?, ..Default::default() };
    r.skip(6)?;
    let l = [r.i16()?, r.i16()?, r.i16()?, r.i16()?];
    c.name = r.string_n(l[0].max(0) as usize, enc)?;
    c.type_name = r.string_n(l[1].max(0) as usize, enc)?;
    c.table_name = r.string_n(l[2].max(0) as usize, enc)?;
    c.schema_name = r.string_n(l[3].max(0) as usize, enc)?;
    if base_name {
        c.base_name = Some(r.u16_string(enc)?);
    }
    if c.item_flag & 2 != 0 {
        c.lob_tab_id = r.i32()?;
        c.lob_col_id = r.i16()?;
    }
    match c.col_type {
        DATETIME | DATETIME2 => {
            if c.scale & 0x1000 != 0 {
                c.local_tz = true;
                c.scale &= !0x1000;
            }
            if c.scale & 0x2000 != 0 {
                c.oracle_date = true;
                c.scale &= !0x2000;
            }
        }
        DECIMAL if c.scale == 129 => {
            c.oracle_float = true;
            c.prec = (c.prec as f64 * 0.30103).round() as i32 + 1;
            c.scale = -1;
        }
        VARCHAR if c.prec == 512 && c.scale == 6 => c.bfile = true,
        ARRAY | CLASS | PLTYPE_RECORD | SARRAY | CURSOR => {
            return Err(ProtoError::Unsupported(format!("complex column type {}", c.col_type)));
        }
        _ => {}
    }
    Ok(c)
}

#[derive(Debug, Clone, Default)]
pub struct PrepareResponse {
    pub sql_type: i16,
    pub params: Vec<ParamDesc>,
    pub columns: Vec<Column>,
    pub trx_status: i32,
}

impl PrepareResponse {
    pub fn parse(f: &Frame, enc: Encoding, base_name: bool) -> Result<Self> {
        let pc = f.u16_at(22)? as usize;
        let cc = f.i16_at(24)?.max(0) as usize;
        let mut r = f.body();
        let mut params = Vec::with_capacity(pc);
        for _ in 0..pc {
            params.push(parse_param(&mut r, enc)?);
        }
        let mut columns = Vec::with_capacity(cc);
        for _ in 0..cc {
            columns.push(parse_column(&mut r, enc, base_name)?);
        }
        Ok(PrepareResponse { sql_type: f.i16_at(20)?, params, columns, trx_status: f.i32_at(34)? })
    }
}

// ---------- EXECUTE ----------

#[derive(Debug, Clone, Copy)]
pub struct ExecParamType {
    /// 0 IN、1 OUT、2 INOUT（规格 §9）。普通查询恒为 0。
    pub io_type: i8,
    pub col_type: i32,
    pub prec: i32,
    pub scale: i32,
}

/// 单个参数值。
#[derive(Debug, Clone)]
pub enum ParamData {
    Null,
    /// 已按目标类型编码的字节
    Inline(Vec<u8>),
    /// 带外 LOB（数据已通过 PUT_DATA 上传）
    OffRow,
    /// OUT 参数，以及 REF CURSOR：请求中不写值（实测，见规格 §16）
    Absent,
}

pub struct ExecOpts {
    pub auto_commit: bool,
    pub max_rows: i64,
    pub query_timeout: i32,
    pub msg_version: u16,
    pub continue_on_error: bool,
    pub batch_allow_max_errors: i32,
}

impl Default for ExecOpts {
    fn default() -> Self {
        ExecOpts { auto_commit: true, max_rows: i64::MAX, query_timeout: 0, msg_version: 8, continue_on_error: false, batch_allow_max_errors: 0 }
    }
}

pub fn exec_request(stmt_id: i32, o: &ExecOpts, types: &[ExecParamType], rows: &[Vec<ParamData>]) -> Result<BytesMut> {
    let mut w = Writer::new(CMD_EXECUTE, stmt_id);
    w.set_u8(20, o.auto_commit as u8);
    w.set_u16(21, types.len() as u16);
    w.set_u8(23, 1);
    w.set_i64(24, if types.is_empty() { 1 } else { rows.len() as i64 });
    w.set_i64(32, 0);
    w.set_i64(40, o.max_rows);
    w.set_u8(48, 1);
    w.set_u8(49, o.continue_on_error as u8);
    w.set_u8(50, 0);
    w.set_u8(51, 0);
    w.set_i32(52, if o.query_timeout == 0 { -1 } else { o.query_timeout });
    w.set_i32(56, o.batch_allow_max_errors);
    w.set_u8(60, 0);
    if !types.is_empty() {
        for t in types {
            w.push_u8(t.io_type as u8);
            w.push_i32(t.col_type);
            w.push_i32(t.prec);
            w.push_i32(t.scale);
        }
        for row in rows {
            if row.len() != types.len() {
                return Err(ProtoError::Invalid("parameter count mismatch".into()));
            }
            for d in row {
                match d {
                    ParamData::Null => w.push_u16(NULL_MARK),
                    ParamData::Inline(b) => {
                        if b.len() > VALUE_MAX_INLINE {
                            return Err(ProtoError::Convert(format!("parameter value too long ({} bytes)", b.len())));
                        }
                        w.push_u16_bytes(b);
                    }
                    ParamData::OffRow => w.push_u16(0),
                    ParamData::Absent => {}
                }
            }
        }
    }
    Ok(w.finish())
}

pub type RawRow = Vec<Option<Bytes>>;

#[derive(Debug, Clone)]
pub enum ExecExtra {
    None,
    Timezone(i16),
    Isolation { level: i16, readonly: bool },
    Schema(String),
    Explain(String),
}

#[derive(Debug, Clone)]
pub struct ExecResponse {
    pub sql_type: i16,
    pub update_count: i64,
    pub rows_in_packet: u32,
    pub rowid: i64,
    pub exec_id: u32,
    pub trx_status: u32,
    pub print_msg: String,
    pub out_params: Vec<Option<Bytes>>,
    pub columns: Vec<Column>,
    pub rows: Vec<RawRow>,
    pub batch_counts: Vec<i64>,
    pub insert_ids: Vec<i64>,
    pub extra: ExecExtra,
    pub bdta: bool,
    /// 列描述是否由本次 EXECUTE 响应携带（否则沿用 PREPARE 的列）
    pub columns_from_response: bool,
}

pub fn parse_rows(r: &mut Reader, ncols: usize, nrows: usize) -> Result<Vec<RawRow>> {
    let mut rows = Vec::with_capacity(nrows.min(65536));
    for _ in 0..nrows {
        r.skip(2)?;
        r.skip(8)?;
        r.skip(2 * ncols)?;
        let mut row = Vec::with_capacity(ncols);
        for _ in 0..ncols {
            let l = r.u16()?;
            row.push(match l {
                NULL_MARK => None,
                LONG_MARK => {
                    let n = r.i32()?;
                    Some(r.bytes(n.max(0) as usize)?)
                }
                n => Some(r.bytes(n as usize)?),
            });
        }
        rows.push(row);
    }
    Ok(rows)
}

impl ExecResponse {
    pub fn parse(f: &Frame, enc: Encoding, base_name: bool, known_cols: &[Column]) -> Result<Self> {
        let sql_type = f.i16_at(20)?;
        let col_count = f.u16_at(22)? as usize;
        let out_cnt = f.u16_at(32)? as usize;
        let rows_in_packet = f.u32_at(35)?;
        let print_len = f.u32_at(39)? as usize;
        let sel = is_select(sql_type);
        let (bdta, rowid) = if sel { (f.u8_at(43)? == 2, 0) } else { (false, f.i64_at(43)?) };
        let flags = f.u8_at(59)?;
        let mut resp = ExecResponse {
            sql_type,
            update_count: f.i64_at(24)?,
            rows_in_packet,
            rowid,
            exec_id: f.u32_at(51)?,
            trx_status: f.u32_at(60)?,
            print_msg: String::new(),
            out_params: vec![],
            columns: vec![],
            rows: vec![],
            batch_counts: vec![],
            insert_ids: vec![],
            extra: ExecExtra::None,
            bdta,
            columns_from_response: false,
        };
        let mut r = f.body();
        if print_len > 0 {
            resp.print_msg = r.string_n(print_len, enc)?;
        }
        for _ in 0..out_cnt {
            let l = r.u16()?;
            resp.out_params.push(match l {
                NULL_MARK => None,
                LONG_MARK => {
                    let n = r.i32()?;
                    Some(r.bytes(n.max(0) as usize)?)
                }
                n => Some(r.bytes(n as usize)?),
            });
        }
        if sel {
            // 实测：EXECUTE(13) 的响应不重复列描述（列数为 0），沿用 PREPARE 返回的列
            if col_count == 0 {
                resp.columns = known_cols.to_vec();
            } else {
                resp.columns_from_response = true;
                for _ in 0..col_count {
                    resp.columns.push(parse_column(&mut r, enc, base_name)?);
                }
            }
            let col_count = resp.columns.len();
            if bdta {
                return Err(ProtoError::Unsupported("BDTA (column-major) result set".into()));
            }
            resp.rows = parse_rows(&mut r, col_count, rows_in_packet as usize)?;
        } else {
            match sql_type {
                SQL_SET_TZ => resp.extra = ExecExtra::Timezone(r.i16()?),
                SQL_SET_ISO => resp.extra = ExecExtra::Isolation { level: r.i16()?, readonly: r.u8()? != 0 },
                SQL_SET_SCHEMA => resp.extra = ExecExtra::Schema(r.i32_string(enc)?),
                SQL_EXPLAIN => resp.extra = ExecExtra::Explain(r.i32_string(enc)?),
                _ => {
                    if flags & 1 != 0 {
                        let n = r.i32()?.max(0) as usize;
                        for _ in 0..n {
                            resp.batch_counts.push(r.i64()?);
                        }
                    }
                    if flags & 2 != 0 {
                        let n = r.i32()?.max(0) as usize;
                        for _ in 0..n {
                            resp.insert_ids.push(r.i64()?);
                        }
                    }
                }
            }
        }
        Ok(resp)
    }
}

// ---------- FETCH ----------

pub fn fetch_request(stmt_id: i32, start: i64, count: i64, result_set_id: i16, prefetch_bytes: i32) -> BytesMut {
    let mut w = Writer::new(CMD_FETCH, stmt_id);
    w.set_i64(20, start);
    w.set_i64(28, count);
    w.set_i16(36, result_set_id);
    w.set_i32(38, prefetch_bytes);
    w.finish()
}

#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub total: i64,
    pub rows: Vec<RawRow>,
}

impl FetchResponse {
    pub fn parse(f: &Frame, ncols: usize) -> Result<Self> {
        let total = f.i64_at(20)?;
        let n = f.u32_at(28)? as usize;
        let mut r = f.body();
        Ok(FetchResponse { total, rows: parse_rows(&mut r, ncols, n)? })
    }
}

// ---------- 错误 ----------

#[derive(Debug, Clone)]
pub struct ServerError {
    pub code: i32,
    pub message: String,
}

pub fn parse_error(f: &Frame, enc: Encoding) -> ServerError {
    let code = f.ret_code();
    let try_parse = || -> Result<String> {
        let mut r = f.body();
        for _ in 0..3 {
            let n = r.i32()?;
            r.skip(n.max(0) as usize)?;
        }
        r.i32_string(enc)
    };
    let message = match try_parse() {
        Ok(m) if !m.is_empty() => m,
        _ => enc.decode(&f.data[crate::wire::HEADER_LEN..]).trim_matches(char::from(0)).to_string(),
    };
    ServerError { code, message }
}

#[derive(Debug, Clone)]
pub struct BatchError {
    pub row: i32,
    pub code: i32,
    pub message: String,
}

pub fn parse_batch_errors(f: &Frame, enc: Encoding) -> Result<Vec<BatchError>> {
    let mut r = f.body();
    r.skip(4)?;
    let n = r.i32()?.max(0);
    let mut v = vec![];
    for _ in 0..n {
        v.push(BatchError { row: r.i32()?, code: r.i32()?, message: r.u16_string(enc)? });
    }
    Ok(v)
}

// ---------- LOB ----------

pub fn lob_length_request(stmt_id: i32, is_clob: bool, h: &LobHead, new_lob: bool) -> BytesMut {
    let mut w = Writer::new(CMD_LOB_LENGTH, stmt_id);
    w.push_u8(is_clob as u8);
    w.push_i64(h.blob_id);
    w.push_i16(h.group_id);
    w.push_i16(h.file_id);
    w.push_i32(h.page_no);
    if new_lob {
        w.push_i32(h.table_id);
        w.push_i16(h.col_id);
        w.push_i64(h.row_id);
        w.push_i16(h.ex_group_id);
        w.push_i16(h.ex_file_id);
        w.push_i32(h.ex_page_no);
    }
    w.finish()
}

pub fn parse_lob_length(f: &Frame) -> Result<i64> {
    let mut r = f.body();
    if r.remaining() >= 8 {
        r.i64()
    } else {
        Ok(r.u32()? as i64)
    }
}

/// LOB 读取游标（跨多次读取推进）。
#[derive(Debug, Clone, Copy, Default)]
pub struct LobCursor {
    pub cur_file_id: i16,
    pub cur_page_no: i32,
    pub total_offset: i32,
}

pub fn lob_read_request(stmt_id: i32, is_clob: bool, h: &LobHead, cur: &LobCursor, offset: i32, length: i32, new_lob: bool) -> BytesMut {
    let mut w = Writer::new(CMD_LOB_READ, stmt_id);
    w.push_u8(is_clob as u8);
    w.push_i32(h.table_id);
    w.push_i16(h.col_id);
    w.push_i64(h.blob_id);
    w.push_i16(h.group_id);
    w.push_i16(h.file_id);
    w.push_i32(h.page_no);
    w.push_i16(cur.cur_file_id);
    w.push_i32(cur.cur_page_no);
    w.push_i32(cur.total_offset);
    w.push_i32(offset);
    w.push_i32(length);
    if new_lob {
        w.push_i64(h.row_id);
        w.push_i16(h.ex_group_id);
        w.push_i16(h.ex_file_id);
        w.push_i32(h.ex_page_no);
    }
    w.finish()
}

#[derive(Debug, Clone)]
pub struct LobReadResponse {
    pub read_over: bool,
    pub data: Bytes,
    pub cursor: LobCursor,
    pub char_len: Option<u32>,
}

pub fn parse_lob_read(f: &Frame) -> Result<LobReadResponse> {
    let mut r = f.body();
    let read_over = r.u8()? == 1;
    let n = r.u32()? as i32;
    if n <= 0 {
        return Ok(LobReadResponse { read_over, data: Bytes::new(), cursor: LobCursor::default(), char_len: None });
    }
    let cursor = LobCursor { cur_file_id: r.i16()?, cur_page_no: r.i32()?, total_offset: r.i32()? };
    let data = r.bytes(n as usize)?;
    let char_len = if r.remaining() >= 4 { Some(r.u32()?) } else { None };
    Ok(LobReadResponse { read_over, data, cursor, char_len })
}

// ---------- 带外 LOB 上传 ----------

/// PRE_EXEC(90)：声明参数类型，之后才能用 PUT_DATA 上传带外 LOB。
pub fn pre_exec_request(stmt_id: i32, types: &[ExecParamType]) -> BytesMut {
    let mut w = Writer::new(CMD_PRE_EXEC, stmt_id);
    w.set_i16(20, types.len() as i16);
    w.set_u8(22, 0);
    for t in types {
        w.push_u8(t.io_type as u8);
        w.push_i32(t.col_type);
        w.push_i32(t.prec);
        w.push_i32(t.scale);
    }
    w.finish()
}

pub fn put_data_request(stmt_id: i32, param_idx: i16, chunk: &[u8], new_lob: bool) -> BytesMut {
    let mut w = Writer::new(CMD_PUT_DATA, stmt_id);
    w.set_i16(20, param_idx);
    w.push_i32(chunk.len() as i32);
    if new_lob {
        w.push_i32(-1);
    }
    w.push_raw(chunk);
    w.finish()
}

pub fn next_result_request(stmt_id: i32, rs_id: i16) -> BytesMut {
    let mut w = Writer::new(CMD_NEXT_RESULT, stmt_id);
    w.set_i16(20, rs_id);
    w.finish()
}

/// 结果集有列但 0 行。正数，不是结束，响应体仍是普通 EXEC 结果。
pub const RC_RESULT_EMPTY: i32 = 100;
/// 没有更多结果集。正数，body 为错误包布局，消息「结果集数据获取完成」。
pub const RC_RESULT_END: i32 = 111;

/// NEXT_RESULT 走到头：返回码 +111，或 sql type 为 0（REF CURSOR 取完时也是 0）。
/// `+100` 配 sql type 160/162 是空结果集，不是结束。
pub fn no_more_results(ret_code: i32, sql_type: i16) -> bool {
    ret_code == RC_RESULT_END || sql_type == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Codec;

    fn frame_of(w: BytesMut) -> Frame {
        Frame::new(w.freeze()).unwrap()
    }

    #[test]
    fn startup_layout() {
        let w = startup_request(&StartupParams {
            compress_mode: 0,
            compress_id: 0,
            login_encrypt: true,
            bdta: false,
            msg_version: 8,
            driver_version: "8.1.4.0",
            dh_public: Some([7u8; 64]),
        });
        let f = frame_of(w);
        assert_eq!(f.cmd(), 200);
        assert_eq!(f.u8_at(28).unwrap(), 1);
        assert_eq!(f.u8_at(29).unwrap(), 2);
        assert_eq!(f.u8_at(34).unwrap(), 1);
        assert_eq!(f.u16_at(35).unwrap(), 8);
        let mut r = f.body();
        assert_eq!(r.i32_bytes().unwrap().as_ref(), b"8.1.4.0");
        assert_eq!(r.u8().unwrap(), 0);
        assert_eq!(r.i32_bytes().unwrap().len(), 64);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn prepare_layout() {
        let o = PrepareOpts { auto_commit: true, direct: false, max_rows: i64::MAX, bdta: false, query_timeout: 0, msg_version: 8 };
        let f = frame_of(prepare_request(9, b"select 1", &o));
        assert_eq!(f.stmt_id(), 9);
        assert_eq!(f.u8_at(20).unwrap(), 1);
        assert_eq!(f.u8_at(21).unwrap(), 0);
        assert_eq!(f.i64_at(27).unwrap(), i64::MAX);
        assert_eq!(&f.data[64..], b"select 1\0");
        assert_eq!(f.i32_at(6).unwrap() as usize, 9);
    }

    #[test]
    fn exec_roundtrip_rows() {
        let types = [ExecParamType { io_type: 0, col_type: INT, prec: 10, scale: 0 }, ExecParamType { io_type: 0, col_type: VARCHAR, prec: 100, scale: 0 }];
        let rows = vec![vec![ParamData::Inline(5i32.to_le_bytes().to_vec()), ParamData::Null], vec![ParamData::Null, ParamData::Inline(b"ab".to_vec())]];
        let f = frame_of(exec_request(3, &ExecOpts::default(), &types, &rows).unwrap());
        assert_eq!(f.u16_at(21).unwrap(), 2);
        assert_eq!(f.i64_at(24).unwrap(), 2);
        let mut r = f.body();
        r.skip(26).unwrap();
        assert_eq!(r.u16().unwrap(), 4);
        assert_eq!(r.i32().unwrap(), 5);
        assert_eq!(r.u16().unwrap(), NULL_MARK);
    }

    #[test]
    fn row_parser() {
        let mut w = Writer::new(7, 1);
        for i in 0..2u8 {
            w.push_u16(0);
            w.push_i64(i as i64);
            w.push_raw(&[0u8; 4]);
            w.push_u16(2);
            w.push_raw(&[i, 0]);
            w.push_u16(NULL_MARK);
        }
        let b = w.finish().freeze();
        let mut r = Reader::new(b, 64);
        let rows = parse_rows(&mut r, 2, 2).unwrap();
        assert_eq!(rows[1][0].as_ref().unwrap().as_ref(), &[1, 0]);
        assert!(rows[0][1].is_none());
    }

    #[test]
    fn error_parse() {
        let mut w = Writer::new(5, 0);
        w.push_i32(2);
        w.push_raw(&[1, 2]);
        w.push_i32(0);
        w.push_i32(1);
        w.push_raw(&[9]);
        w.push_i32_bytes("语法分析出错".as_bytes());
        let mut b = w.finish();
        b[10..14].copy_from_slice(&(-2007i32).to_le_bytes());
        let f = Frame::new(b.freeze()).unwrap();
        let e = parse_error(&f, Encoding::Utf8);
        assert_eq!(e.code, -2007);
        assert_eq!(e.message, "语法分析出错");
        let _ = Codec::default();
    }

    #[test]
    fn exec_out_inout_and_cursor_omit_values() {
        let types = [
            ExecParamType { io_type: 0, col_type: INT, prec: 4, scale: 0 },
            ExecParamType { io_type: 1, col_type: INT, prec: 4, scale: 0 },
            ExecParamType { io_type: 2, col_type: VARCHAR, prec: 50, scale: 0 },
            ExecParamType { io_type: 1, col_type: CURSOR, prec: 32768, scale: 0 },
        ];
        let row = vec![
            ParamData::Inline(3i32.to_le_bytes().to_vec()),
            ParamData::Absent,
            ParamData::Inline(b"ab".to_vec()),
            ParamData::Absent,
        ];
        let f = frame_of(exec_request(8, &ExecOpts::default(), &types, &[row]).unwrap());
        assert_eq!(f.u16_at(21).unwrap(), 4);
        let mut r = f.body();
        assert_eq!(r.u8().unwrap(), 0);
        assert_eq!(r.i32().unwrap(), INT);
        assert_eq!(r.i32().unwrap(), 4);
        assert_eq!(r.i32().unwrap(), 0);
        assert_eq!(r.u8().unwrap(), 1);
        assert_eq!(r.i32().unwrap(), INT);
        r.skip(8).unwrap();
        assert_eq!(r.u8().unwrap(), 2);
        assert_eq!(r.i32().unwrap(), VARCHAR);
        r.skip(8).unwrap();
        assert_eq!(r.u8().unwrap(), 1);
        assert_eq!(r.i32().unwrap(), CURSOR);
        assert_eq!(r.i32().unwrap(), 32768);
        assert_eq!(r.i32().unwrap(), 0);
        // IN 的 i32，然后 INOUT 的字符串；两个 OUT（含 CURSOR）都没有值字节。
        assert_eq!(r.u16().unwrap(), 4);
        assert_eq!(r.i32().unwrap(), 3);
        assert_eq!(r.u16().unwrap(), 2);
        assert_eq!(r.bytes(2).unwrap().as_ref(), b"ab");
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn exec_response_out_params_before_result() {
        let mut w = Writer::new(13, 4);
        w.set_i16(20, SQL_SELECT2);
        w.set_u16(22, 0);
        w.set_i64(24, 0);
        w.set_u16(32, 2);
        w.push_u16(4);
        w.push_i32(13);
        w.push_u16(NULL_MARK);
        let f = frame_of(w.finish());
        let er = ExecResponse::parse(&f, Encoding::Utf8, false, &[]).unwrap();
        assert_eq!(er.sql_type, SQL_SELECT2);
        assert_eq!(er.out_params.len(), 2);
        assert_eq!(er.out_params[0].as_ref().unwrap().as_ref(), 13i32.to_le_bytes());
        assert!(er.out_params[1].is_none());
        assert!(er.columns.is_empty());
        assert!(er.rows.is_empty());
    }

    #[test]
    fn next_result_layout_and_end_marker() {
        let f = frame_of(next_result_request(9, 0));
        assert_eq!(f.cmd(), CMD_NEXT_RESULT);
        assert_eq!(f.stmt_id(), 9);
        assert_eq!(f.i16_at(20).unwrap(), 0);
        assert_eq!(f.i32_at(6).unwrap(), 0);
        let f = frame_of(next_result_request(9, 2));
        assert_eq!(f.i16_at(20).unwrap(), 2);
        assert!(no_more_results(RC_RESULT_END, 0));
        assert!(no_more_results(0, 0));
        assert!(!no_more_results(RC_RESULT_EMPTY, SQL_SELECT2));
        assert!(!no_more_results(0, SQL_SELECT));
        assert!(!no_more_results(0, SQL_SELECT2));
        assert!(!no_more_results(0, SQL_INSERT));
    }

    #[test]
    fn exec_response_update_count_without_columns() {
        // 多条 DML 拼成一条 SQL 时，响应 sql type 仍是 162，但没有列、没有行，@24 是更新计数。
        let mut w = Writer::new(CMD_EXECUTE, 3);
        w.set_i16(20, SQL_SELECT2);
        w.set_u16(22, 0);
        w.set_i64(24, 2);
        let f = frame_of(w.finish());
        let er = ExecResponse::parse(&f, Encoding::Utf8, false, &[]).unwrap();
        assert_eq!(er.sql_type, SQL_SELECT2);
        assert!(er.columns.is_empty());
        assert!(er.rows.is_empty());
        assert!(!er.columns_from_response);
        assert_eq!(er.update_count, 2);
        assert!(!no_more_results(0, er.sql_type));
    }
}
