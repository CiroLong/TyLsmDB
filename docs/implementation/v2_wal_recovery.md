# V2 WAL 与恢复实现计划

> **给智能代理执行者：** 必须使用子技能 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐任务执行本计划。步骤使用复选框（`- [ ]`）语法便于跟踪。

**目标：** 通过 append-only WAL 让 V1 的写入具备持久性，并在重新打开数据库时 replay WAL。

**架构：** `DB::write` 先把每个 `WriteBatch` 编码成 WAL record，再应用到 mutable memtable。恢复流程按 sequence 顺序 replay 有效 record，并忽略文件末尾的 torn record。

**技术栈：** Rust 2024、标准库文件系统 API、当前版本引入的 CRC32C crate、`cargo test`。

---

## 范围

本版本包含：

- 增加 varint 编解码工具。
- 增加带 checksum、length、record type、payload 的 WAL record 格式。
- 编码/解码带起始 sequence 的 `WriteBatch`。
- 在 DB 目录创建 `000001.wal`。
- `DB::open` 时 replay WAL record。
- 实现 `WriteOptions.sync` 和 `DB::sync_wal`。

本版本不包含：

- 除非单 batch 体积测试需要，否则暂不实现多 fragment WAL record。
- 不实现 group commit。
- 不实现 WAL recycling。
- 不实现依赖 MANIFEST 的 WAL 发现。

## 文件划分

- 修改：`Cargo.toml`
- 修改：`src/util/coding.rs`
- 创建：`src/util/crc.rs`
- 创建：`src/wal/mod.rs`
- 创建：`src/wal/format.rs`
- 创建：`src/wal/writer.rs`
- 创建：`src/wal/reader.rs`
- 修改：`src/batch.rs`
- 修改：`src/db.rs`
- 修改：`src/lib.rs`
- 测试：`src/wal/*` 模块测试和 `src/db.rs` reopen 测试

## 任务

- [ ] **步骤 1：增加 checksum 依赖**

  在 `Cargo.toml` 中增加：

  ```toml
  [dependencies]
  crc32c = "0.6"
  ```

- [ ] **步骤 2：实现 varint 编解码**

  在 `src/util/coding.rs` 中实现：

  - `put_var_u32(dst: &mut Vec<u8>, value: u32)`
  - `put_var_u64(dst: &mut Vec<u8>, value: u64)`
  - `get_var_u32(src: &mut &[u8]) -> Result<u32>`
  - `get_var_u64(src: &mut &[u8]) -> Result<u64>`

  为 `0`、`1`、`127`、`128`、`16384`、`u64::MAX` 增加 roundtrip 测试。

- [ ] **步骤 3：定义 WAL 格式**

  在 `src/wal/format.rs` 中定义：

  ```rust
  pub const WAL_RECORD_HEADER_SIZE: usize = 9;

  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum WalRecordType {
      Full = 1,
  }
  ```

  V2 只写 `Full` record。等引擎真正需要大于 WAL block 策略的 record 时，再增加 fragment 类型。

- [ ] **步骤 4：编码 WriteBatch payload**

  给 `WriteBatch` 增加方法：

  - `encode_with_sequence(start_sequence: u64) -> Vec<u8>`
  - `decode_payload(payload: &[u8]) -> Result<(u64, WriteBatch)>`

  Payload 布局：

  ```text
  batch_count: varint32
  start_sequence: varint64
  repeated:
    value_type: u8
    key_len: varint32
    key
    value_len: varint32
    value
  ```

  Delete record 编码时使用 `value_len = 0`。

- [ ] **步骤 5：实现 WAL writer**

  `WalWriter` 持有一个 append 模式打开的 `File`，写入格式为：

  ```text
  crc32c: u32 little-endian
  length: u32 little-endian
  type: u8
  payload
  ```

  方法：

  - `create(path) -> Result<WalWriter>`
  - `append(payload) -> Result<()>`
  - `sync() -> Result<()>`

- [ ] **步骤 6：实现 WAL reader**

  `WalReader` 顺序读取 record 并校验 checksum。干净 EOF、末尾不完整 header、末尾不完整 payload 都返回 `Ok(None)`；如果完整长度的 record checksum 不匹配，则返回 `Error::Corruption`。

- [ ] **步骤 7：把 WAL 接入 DB open 和 write**

  `DB::open` 必须：

  - 当 `Options::create_if_missing` 为 true 时创建 DB 目录。
  - 如果 `000001.wal` 存在，则 replay 它。
  - 创建或追加打开 `000001.wal`。

  `DB::write` 必须：

  - 分配 sequence。
  - 除非 `WriteOptions.disable_wal` 为 true，否则 append WAL。
  - 当 `WriteOptions.sync` 为 true，或 `options.wal_sync == WalSyncMode::PerWrite` 时执行 sync。
  - WAL append 成功后再把 record 应用到 memtable。

- [ ] **步骤 8：增加恢复测试**

  在 `src/db.rs` 增加集成风格测试：

  - `reopen_replays_puts_and_deletes`
  - `sync_wal_flushes_file`
  - `trailing_partial_wal_record_is_ignored`
  - `corrupt_complete_wal_record_returns_error`

  每个测试使用 `target/tylsmdb-tests` 下的独立目录。

- [ ] **步骤 9：验证版本**

  运行：`cargo fmt`

  期望：命令成功退出。

  运行：`cargo test`

  期望：全部测试通过，包括 reopen 测试。

## 退出条件

- 已 sync 的写入在 `drop(db)` 并重新 `DB::open` 后仍存在。
- 文件末尾 torn WAL record 不会阻止数据库打开。
- 完整但损坏的 WAL record 会被报告为 corruption。
- V1 的内存语义在恢复后保持不变。

## 建议提交

```bash
git add Cargo.toml src
git commit -m "feat: add WAL persistence and recovery"
```
