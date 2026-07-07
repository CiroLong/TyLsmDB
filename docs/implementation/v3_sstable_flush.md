# V3 SSTable 与刷盘实现计划

> **给智能代理执行者：** 必须使用子技能 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐任务执行本计划。步骤使用复选框（`- [ ]`）语法便于跟踪。

**目标：** 增加 immutable memtable，把它们 flush 成 SSTable，并能读回已 flush 的数据。

**架构：** mutable memtable 超过 `Options.memtable_size` 后冻结。flush 路径构建单个 L0 table file；在 V4 引入 VersionSet 前，先用临时的内存列表保存 flushed table。

**技术栈：** Rust 2024、标准库文件系统 API、V2 的 CRC32C、`cargo test`。

---

## 范围

本版本包含：

- 带前缀压缩与 restart point 的 data block builder。
- SSTable builder 与 reader。
- Block iterator 与 table iterator。
- 手动 flush 和 size-triggered flush。
- 横跨 mutable memtable、immutable memtable、flushed L0 table 的读路径。

本版本不包含：

- 不实现 MANIFEST 或持久化 table metadata。
- 不实现 Bloom filter。
- 不实现 `NoCompression` 之外的压缩。
- 不实现后台 worker；`flush()` 可以同步执行。

## 文件划分

- 创建：`src/table/mod.rs`
- 创建：`src/table/format.rs`
- 创建：`src/table/block.rs`
- 创建：`src/table/block_builder.rs`
- 创建：`src/table/block_iterator.rs`
- 创建：`src/table/builder.rs`
- 创建：`src/table/reader.rs`
- 创建：`src/table/properties.rs`
- 修改：`src/db.rs`
- 修改：`src/memtable/btree.rs`
- 修改：`src/lib.rs`
- 测试：`src/table/*` 模块测试和 `src/db.rs` flush 测试

## 任务

- [ ] **步骤 1：定义 table 格式常量**

  在 `src/table/format.rs` 中定义：

  - `TABLE_MAGIC: u64`
  - `FOOTER_SIZE`
  - `CompressionType::None`
  - `BlockHandle { offset: u64, size: u64 }`

  为 `BlockHandle` 增加 encode/decode 测试。

- [ ] **步骤 2：实现 block builder**

  `BlockBuilder` 接收已排序的 `(InternalKey, ValueRecord)`，编码为：

  ```text
  shared_key_len: varint32
  unshared_key_len: varint32
  value_len: varint32
  value_type: u8
  unshared_internal_key
  value
  restart_offsets: [u32]
  restart_count: u32
  ```

  默认 restart interval 为 16 条 entry。

- [ ] **步骤 3：实现 block iterator**

  `BlockIterator` 必须支持：

  - seek to first。
  - 使用 restart point seek 到 internal key。
  - 按顺序返回 key/value record。
  - 对格式错误的 block 返回 `Error::Corruption`。

- [ ] **步骤 4：实现 SSTable builder**

  `SSTableBuilder` 写入：

  ```text
  data blocks
  index block
  properties block
  footer
  ```

  每个 block 后追加：

  ```text
  compression_type: u8
  crc32c: u32
  ```

- [ ] **步骤 5：实现 SSTable reader**

  `SSTableReader::open(path)` 读取 footer、index block 和 properties。必需方法：

  - `get(user_key, read_seq) -> Result<Option<ValueRecord>>`
  - `iter() -> TableIterator`
  - `smallest_key()`
  - `largest_key()`

- [ ] **步骤 6：增加 immutable memtable 状态**

  扩展 DB state：

  ```rust
  mutable: MemTable,
  immutables: Vec<MemTable>,
  l0_tables: Vec<Arc<SSTableReader>>,
  next_file_number: u64,
  ```

  mutable table 超过 `memtable_size` 后冻结，并创建新的 mutable table。

- [ ] **步骤 7：实现同步 flush**

  `DB::flush` 必须：

  - 移出最老的 immutable memtable；如果没有 immutable，则移动当前 mutable memtable。
  - 写入 `00000N.sst.tmp`。
  - sync 文件。
  - rename 为 `00000N.sst`。
  - 打开 reader，并按 newest first 插入 `l0_tables`。

- [ ] **步骤 8：扩展读路径**

  `DB::get` 检查顺序：

  1. Mutable memtable。
  2. Immutable memtables newest to oldest。
  3. L0 tables newest to oldest。

  `DB::scan` 合并三个来源的可见 record，并为每个 key 返回一个用户可见值。

- [ ] **步骤 9：增加 table 与 flush 测试**

  测试：

  - `block_roundtrip_with_prefix_compression`
  - `table_builder_reader_roundtrip`
  - `flush_moves_memtable_to_l0_table`
  - `get_reads_from_flushed_table`
  - `scan_merges_memtable_and_table_versions`

- [ ] **步骤 10：验证版本**

  运行：`cargo fmt`

  期望：命令成功退出。

  运行：`cargo test`

  期望：全部测试通过。

## 退出条件

- flush 后的数据在进程退出前可读。
- block checksum 校验能拒绝损坏的 table block。
- 读顺序为 mutable、immutable、L0 newest to oldest。
- SST metadata 的 reopen 持久性明确推迟到 V4。

## 建议提交

```bash
git add src
git commit -m "feat: add SSTable format and synchronous flush"
```
