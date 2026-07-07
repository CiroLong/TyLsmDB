# V5 读路径与缓存实现计划

> **给智能代理执行者：** 必须使用子技能 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐任务执行本计划。步骤使用复选框（`- [ ]`）语法便于跟踪。

**目标：** 完整打通 memtable、L0 和低层非重叠文件的读路径，并增加生产可用的 iterator、Bloom filter 和 cache。

**架构：** 读请求 clone 一个不可变 state snapshot，搜索 table 时不持有全局锁。所有存储来源通过 `StorageIterator` trait 统一，`DBIterator` 负责把内部多版本记录过滤为用户可见记录。

**技术栈：** Rust 2024、V3 的 block/table reader、V4 的 VersionSet、当前版本选用的 cache crate、`cargo test`。

---

## 范围

本版本包含：

- `StorageIterator`、`MergeIterator`、`TwoMergeIterator`、`ConcatIterator`、`DBIterator`。
- 对重叠 L0 文件按 newest to oldest 搜索。
- 对 L1+ 的非重叠文件范围进行查找。
- Table-level Bloom filter。
- Block cache 和 table cache。

本版本不包含：

- 不实现生成 L1+ 文件的 compaction；测试可以直接构造 version。
- 不实现 partitioned index 或 partitioned filter。
- 不实现 async I/O。

## 文件划分

- 创建：`src/iterator/mod.rs`
- 创建：`src/iterator/storage_iterator.rs`
- 创建：`src/iterator/merge_iterator.rs`
- 创建：`src/iterator/two_merge_iterator.rs`
- 创建：`src/iterator/concat_iterator.rs`
- 创建：`src/iterator/db_iterator.rs`
- 创建：`src/table/filter.rs`
- 创建：`src/util/bloom.rs`
- 创建：`src/cache/mod.rs`
- 创建：`src/cache/block_cache.rs`
- 创建：`src/cache/table_cache.rs`
- 修改：`src/table/builder.rs`
- 修改：`src/table/reader.rs`
- 修改：`src/db.rs`
- 修改：`src/lib.rs`
- 修改：`Cargo.toml`

## 任务

- [ ] **步骤 1：增加 cache 依赖**

  选择一个 cache 实现，并加入 `Cargo.toml`：

  ```toml
  moka = { version = "0.12", features = ["sync"] }
  ```

- [ ] **步骤 2：定义 StorageIterator trait**

  必需方法：

  - `is_valid() -> bool`
  - `key() -> &InternalKey`
  - `value() -> &ValueRecord`
  - `next() -> Result<()>`
  - `seek(key: &InternalKey) -> Result<()>`

- [ ] **步骤 3：实现归并 iterator**

  `MergeIterator` 按 internal key 合并 N 个有序 iterator。如果两个来源有相同 internal key，source index 更小的一侧胜出，以保持来源优先级。`TwoMergeIterator` 是合并 memtable 与 table iterator 的轻量封装。`ConcatIterator` 顺序串接同层非重叠 table iterator。

- [ ] **步骤 4：实现 DBIterator**

  `DBIterator` 消费 internal key，对外暴露 `(user_key, value)`：

  - 跳过 `sequence > read_seq` 的 record。
  - 每个 user key 只返回最新可见 record。
  - 如果最新可见 record 是 tombstone，则隐藏该 key。
  - 遵守 lower/upper user-key bound。

- [ ] **步骤 5：增加 table-level Bloom filter**

  SSTable 构建时从 user key 生成 filter。Reader 做点查时先查 filter，再决定是否打开 data block。Bloom negative 绝不能隐藏真实存在的 key。

- [ ] **步骤 6：增加 block cache**

  `BlockCache` key：

  ```text
  table_file_number + block_offset
  ```

  Cache value：已解码的不可变 block。遵守 `ReadOptions.fill_cache`；当 fill 为 false 时，点查可以绕过 cache。

- [ ] **步骤 7：增加 table cache**

  `TableCache` 把 file number 映射到 `Arc<SSTableReader>`。它与 `Options.block_cache_capacity` 分开控制；V5 可先固定 table cache 容量为 512 entries。

- [ ] **步骤 8：补完整 DB 读路径**

  `DB::get` 必须搜索：

  1. Mutable memtable。
  2. Immutable memtables newest to oldest。
  3. L0 overlapping files newest to oldest。
  4. L1+ 每层最多一个 overlapping file。

  `DB::scan` 必须构造所有来源 iterator，并通过 `DBIterator` 输出。

- [ ] **步骤 9：增加读路径测试**

  测试：

  - `db_iterator_filters_old_versions_and_tombstones`
  - `l0_get_uses_newest_overlapping_file`
  - `lower_level_get_searches_one_file_per_level`
  - `scan_merges_memtables_l0_and_levels`
  - `bloom_filter_has_no_false_negatives`
  - `block_cache_records_hits_after_repeated_get`

- [ ] **步骤 10：验证版本**

  运行：`cargo fmt`

  期望：命令成功退出。

  运行：`cargo test`

  期望：全部测试通过。

## 退出条件

- 点查与 scan 使用统一的可见性实现。
- L0 overlap 语义和 L1+ non-overlap 语义都有测试覆盖。
- Bloom filter 只在返回 negative 时跳过工作。
- 重复读取能命中 block cache。

## 建议提交

```bash
git add Cargo.toml src
git commit -m "feat: complete read iterators and table caches"
```
