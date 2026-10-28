//! GB18030 实例（UNICODE_FLAG=0）真机测试。默认连 127.0.0.1:5237，可用 DM_TEST_GB_URL 覆盖；连不上则跳过。
//! 起实例：docker run -d --name dm8gb -p 5237:5236 -e UNICODE_FLAG=0 ... dm8_single:<tag>
use dm_conn::{ConnectOptions, Connection, Encoding};

#[tokio::test]
async fn gb18030_instance_roundtrip() {
    let url = std::env::var("DM_TEST_GB_URL").unwrap_or_else(|_| "dm://SYSDBA:SYSDBA001@127.0.0.1:5237".into());
    let mut c = match Connection::connect(&ConnectOptions::from_url(&url).unwrap()).await {
        Ok(c) => c,
        Err(e) => return eprintln!("skip: {e}"),
    };
    if c.encoding() != Encoding::Gb18030 {
        return eprintln!("skip: instance is not GB18030 ({:?})", c.encoding());
    }
    // 字面量与 4 字节 GB18030 字符（扩展 B 区汉字、emoji）
    let r = c.query("select '中文测试𠀀€' as a, lengthb('中文') as b from dual", &[]).await.unwrap();
    assert_eq!(r.rows[0].values[0].as_str().unwrap(), "中文测试𠀀€");
    assert_eq!(r.rows[0].values[1].to_text().unwrap(), "4"); // GB18030 每个汉字 2 字节

    let _ = c.execute("drop table \"表_GB\"", &[]).await;
    c.execute("create table \"表_GB\"(id int, \"名称\" varchar(100), cl clob)", &[]).await.unwrap();
    c.execute("comment on table \"表_GB\" is '中文注释'", &[]).await.unwrap();
    let s = "汉字ａ€😀";
    let long = "长文本中文".repeat(1000); // > 2048 字节，走 LOB 带外写入
    c.execute("insert into \"表_GB\" values(?,?,?)", &[1.into(), s.into(), long.clone().into()]).await.unwrap();
    let r = c.query("select \"名称\", cl from \"表_GB\" where \"名称\" = ?", &[s.into()]).await.unwrap();
    assert_eq!(r.columns[0].name, "名称");
    assert_eq!(r.rows[0].values[0].as_str().unwrap(), s);
    assert_eq!(r.rows[0].values[1].as_str().unwrap(), long);
    let r = c.query("select COMMENTS from USER_TAB_COMMENTS where TABLE_NAME = ?", &["表_GB".into()]).await.unwrap();
    assert_eq!(r.rows[0].values[0].as_str().unwrap(), "中文注释");
    // 服务端中文错误消息按 GB18030 解码
    let e = c.query("select * from \"不存在的表\"", &[]).await.unwrap_err().to_string();
    assert!(e.contains("不存在的表"), "{e}");
    c.execute("drop table \"表_GB\"", &[]).await.unwrap();
}
