# ShellAX 对象树元数据 SQL（DM8 实测）

实测环境：本机 docker `dm8`（DM8 8.1.4.6，UTF-8 实例，SYSDBA 登录），驱动 dm-conn。
测试：`crates/dm-conn/tests/metadata.rs`（`cargo test -p dm-conn --test metadata`）。
所有标识符默认大写存储，传参时用大写；`?` 为绑定参数。

## 1. 模式

DM 里**用户 ≠ 模式**：一个用户可拥有多个模式（`create schema S2 authorization U`），
`ALL_USERS` / `DBA_USERS` 只列用户，**看不到非同名模式**。模式列表用 SYSOBJECTS：

```sql
select s.NAME as SCHEMA_NAME, u.NAME as OWNER
from SYSOBJECTS s join SYSOBJECTS u on s.PID = u.ID
where s.TYPE$ = 'SCH' order by s.NAME
```
示例：`CTISYS|SYS`、`META_S2|META_T`、`META_T|META_T`、`SYS|SYS`、`SYSAUDITOR|SYSAUDITOR`、`SYSDBA|SYSDBA`、`SYSSSO|SYSSSO`。
（`SYSOBJECTS.ID`: INT；只要模式名可用 `select NAME from SYSOBJECTS where TYPE$='SCH'`。）

备选（同样包含全部模式，但 OWNER 列是模式名本身，不是属主）：
`select OBJECT_NAME from DBA_OBJECTS where OBJECT_TYPE='SCH'`。
不推荐：`ALL_USERS(USERNAME, USER_ID)`、`DBA_USERS(USERNAME, ACCOUNT_STATUS…)`、`select distinct OWNER from ALL_OBJECTS`——都漏掉不同名模式。
当前模式：`select SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from dual` → `SYSDBA`。

## 2. 表

```sql
select OWNER, TABLE_NAME, TABLESPACE_NAME, NUM_ROWS from ALL_TABLES where OWNER = ? order by TABLE_NAME
```
列：OWNER VARCHAR、TABLE_NAME VARCHAR、TABLESPACE_NAME VARCHAR、NUM_ROWS DEC（未收集统计时 NULL）。
示例：`META_T | EMP | MAIN | NULL`。`OWNER` 就是**模式名**（非同名模式 `META_S2` 下的表 OWNER=`META_S2`）。

## 3. 视图

```sql
select OWNER, VIEW_NAME, TEXT from ALL_VIEWS where OWNER = ? order by VIEW_NAME
```
TEXT 为 **CLOB**，示例：`" SELECT ID,NAME FROM META_T.EMP"`（服务端规范化后的文本，带前导空格）。

## 4. 字段

```sql
select COLUMN_ID, COLUMN_NAME, DATA_TYPE, DATA_LENGTH, DATA_PRECISION, DATA_SCALE, NULLABLE, DATA_DEFAULT
from ALL_TAB_COLUMNS where OWNER = ? and TABLE_NAME = ? order by COLUMN_ID
```
COLUMN_ID/DATA_LENGTH/DATA_PRECISION/DATA_SCALE 为 DEC，NULLABLE 为 `Y`/`N`，DATA_DEFAULT 为 VARCHAR。示例：
```
1 ID      INT     4  NULL 0 N NULL
2 NAME    VARCHAR 50 NULL 0 N NULL
3 DEPT_ID INT     4  NULL 0 Y NULL
4 SAL     DECIMAL 22 10   2 Y 0
```
视图字段同样适用（TABLE_NAME 传视图名）。

## 5. 索引

```sql
-- 用户可见索引（排除聚簇索引 CLUSTER 和主键/唯一约束自动建的 INDEXnnnn）
select i.INDEX_NAME, i.INDEX_TYPE, i.UNIQUENESS
from ALL_INDEXES i
where i.TABLE_OWNER = ? and i.TABLE_NAME = ? and i.INDEX_TYPE <> 'CLUSTER'
  and not exists (select 1 from ALL_CONSTRAINTS c where c.OWNER = i.OWNER and c.INDEX_NAME = i.INDEX_NAME)
order by i.INDEX_NAME

select INDEX_NAME, COLUMN_NAME, COLUMN_POSITION, DESCEND
from ALL_IND_COLUMNS where TABLE_OWNER = ? and TABLE_NAME = ? order by INDEX_NAME, COLUMN_POSITION
```
不加过滤时 `ALL_INDEXES` 会多出 `INDEX33556026 CLUSTER NONUNIQUE`（表的聚簇索引）和
`INDEX33556027 NORMAL UNIQUE`（主键索引）。示例：`IDX_EMP_DEPT NORMAL NONUNIQUE`、`UX_EMP_NAME NORMAL UNIQUE`；
列：`IDX_EMP_DEPT DEPT_ID 1 ASC`、`IDX_EMP_DEPT NAME 2 ASC`。UNIQUENESS 取 `UNIQUE`/`NONUNIQUE`，DESCEND 取 `ASC`/`DESC`。

## 6. 主键 / 约束

```sql
select CONSTRAINT_NAME, CONSTRAINT_TYPE, INDEX_NAME, STATUS from ALL_CONSTRAINTS where OWNER = ? and TABLE_NAME = ?

select c.CONSTRAINT_NAME, cc.COLUMN_NAME, cc.POSITION
from ALL_CONSTRAINTS c join ALL_CONS_COLUMNS cc on c.OWNER = cc.OWNER and c.CONSTRAINT_NAME = cc.CONSTRAINT_NAME
where c.OWNER = ? and c.TABLE_NAME = ? and c.CONSTRAINT_TYPE = 'P' order by cc.POSITION
```
示例：`CONS134219044 | P | INDEX33556027 | ENABLED`；主键列 `CONS134219044 | ID | 1`。
CONSTRAINT_TYPE：P 主键、U 唯一、R 外键、C 检查（与 Oracle 一致；本次只建了主键）。

## 7. 注释

```sql
select OWNER, TABLE_NAME, TABLE_TYPE, COMMENTS from ALL_TAB_COMMENTS where OWNER = ? order by TABLE_NAME
select TABLE_NAME, COLUMN_NAME, COMMENTS from ALL_COL_COMMENTS where OWNER = ? and TABLE_NAME = ?
```
表和视图都会列出（TABLE_TYPE = `TABLE`/`VIEW`），无注释为 NULL。示例：`META_T | EMP | TABLE | 员工表`、`EMP | NAME | 姓名`。

## 8. DDL：DBMS_METADATA.GET_DDL（返回 CLOB，驱动直接得到字符串）

```sql
select DBMS_METADATA.GET_DDL('TABLE', ?, ?) from dual   -- (对象名, 模式名)
select DBMS_METADATA.GET_DDL('VIEW',  ?, ?) from dual
select DBMS_METADATA.GET_DDL('INDEX', ?, ?) from dual
```
返回列类型名 `TEXT`。示例（换行为 `\r\n`）：
```
CREATE TABLE "META_T"."EMP"
(
"ID" INT NOT NULL,
"NAME" VARCHAR(50) NOT NULL,
"DEPT_ID" INT,
"SAL" DECIMAL(10,2) DEFAULT 0,
NOT CLUSTER PRIMARY KEY("ID")) STORAGE(ON "MAIN", CLUSTERBTR) ;

create OR REPLACE  view META_T.V_EMP as select ID, NAME from META_T.EMP
CREATE OR REPLACE  INDEX "META_T"."IDX_EMP_DEPT" ON "META_T"."EMP"("DEPT_ID" ASC,"NAME" ASC) STORAGE(ON "MAIN", CLUSTERBTR) ;
```
注意：表 DDL **不含**表/列注释和非约束索引，需要的话自行拼 `COMMENT ON …` 与各索引 DDL。
对象不存在或是系统自动建的内部索引（如 `INDEX33556027`）→ 错误 **-20008**「未找到对象或不允许查询系统定义的内部索引」。

## 9. 不可用 / 注意事项

- 本次所有列出的视图和函数都可用，没有遇到不存在的字典视图。
- 模式不要用 ALL_USERS/DBA_USERS（见 §1）。
- 自动生成的索引/约束名形如 `INDEXnnnnnnnn`、`CONSnnnnnnnnn`，数字每次建表都不同，测试里不要写死。
- 普通用户登录时 ALL_* 只返回有权限的对象；SYSOBJECTS 需对系统表的查询权限（SYSDBA 默认有）。
