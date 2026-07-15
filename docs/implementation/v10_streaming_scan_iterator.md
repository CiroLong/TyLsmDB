# V10 流式 Scan Iterator 实现计划

> **给智能代理执行者：** 必须使用子技能 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐任务执行本计划。步骤使用复选框（`- [ ]`）语法便于跟踪。

**目标：** 在保留现有 `scan -> Vec` 兼容 API 的前提下，新增公开的流式 scan iterator，并让 SSTable scan 路径按 block 懒加载。

**架构：** V10 复用已有 `StorageIterator`、`MergeIterator` 和 `DBIterator` 可见性过滤层。新增 `DB::scan_iter` / `DB::scan_iter_opt` 返回 cursor-style `DBIterator`，同时增加 SSTable 级 `StorageIterator`，让 DB scan iterator 不再把每个 SSTable 预先展开成完整 `Vec`。旧的 `DB::scan` 继续通过 iterator `collect()` 得到 `Vec`，保持调用方兼容。

**技术栈：** Rust 2024、现有 iterator 模块、SSTable block/index、block cache、`cargo test`、`cargo clippy`、`cargo fmt`。

---

## 范围

本版本包含：

- 新增 `DB::scan_iter(lower, upper) -> Result<DBIterator>`。
- 新增 `DB::scan_iter_opt(lower, upper, ReadOptions) -> Result<DBIterator>`。
- 从 crate 根导出 `DBIterator`，让用户可以直接使用公开 iterator 类型。
- 为 `DBIterator` 增加 fallible 构造路径，使 lower bound seek 失败时能向调用方返回错误。
- 增加 SSTable scan 用的懒加载 `StorageIterator`，按 block 读取，不在构造 scan 时展开整张表。
- 保留 `SSTableReader::entries()` / `entries_with_cache()`，供 compaction、测试和旧路径使用。
- 更新 `DB::scan` / `scan_opt`，让它们复用新的 `scan_iter` 后再 `collect()`。
- 更新 README、usage 和实现路线图，移除“scan 非流式”的旧限制说明。
- 增加 V10 回归测试，覆盖公开 iterator、snapshot 可见性、lower-bound 懒加载和 `fill_cache = false`。

本版本不包含：

- 不把事务 scan 改成返回事务 iterator；事务内部仍可复用 `DB::scan_opt` 的收集结果记录 range read。
- 不实现后台 flush/compaction worker。
- 不改变 `DB::scan` 既有返回类型。
- 不新增 async iterator 或外部 object-store Env。

## 文件划分

- 修改：`src/lib.rs`
- 修改：`src/db.rs`
- 修改：`src/iterator/db_iterator.rs`
- 修改：`src/table/reader.rs`
- 新增：`tests/streaming_scan_iterator.rs`
- 修改：`docs/implementation/README.md`
- 修改：`docs/usage.md`
- 修改：`README.md`

## 任务

- [x] **步骤 1：增加公开 scan iterator API 的红灯测试**

  新增 `tests/streaming_scan_iterator.rs`，先写测试：

  - `scan_iter_returns_visible_rows_incrementally`
  - `scan_iter_snapshot_preserves_old_versions`
  - `scan_iter_lower_bound_does_not_load_all_prefix_blocks`
  - `scan_iter_fill_cache_false_does_not_touch_block_cache`

  期望在未实现 API 时编译失败，证明测试先于实现。

- [x] **步骤 2：暴露 `DBIterator` 并新增 DB API**

  修改 `src/lib.rs` 导出 `DBIterator`。在 `src/db.rs` 新增：

  - `DB::scan_iter`
  - `DB::scan_iter_opt`

  `scan` 和 `scan_opt` 改为调用 `scan_iter_opt(...).and_then(|mut iter| iter.collect())`。

- [x] **步骤 3：让 `DBIterator` 支持 fallible lower-bound seek**

  在 `src/iterator/db_iterator.rs` 中新增 `try_new(...) -> Result<Self>`，根据 lower bound 对内部 iterator 执行初始 seek：

  - `Included(k)` seek 到 `InternalKey(k, u64::MAX, ValueType::Put)`。
  - `Excluded(k)` seek 到 `InternalKey(k, 0, ValueType::Delete)`。
  - `Unbounded` 不 seek。

  保留 `new(...) -> Self` 兼容现有测试和内部调用。

- [x] **步骤 4：增加 SSTable 懒加载 scan iterator**

  在 `src/table/reader.rs` 增加 SSTable 级 `StorageIterator`：

  - 持有 `Arc<SSTableReader>`、table number、可选 `BlockCache` clone、`fill_cache`、当前 block 和 block 内下标。
  - `next()` 只在跨 block 时读取下一个 data block。
  - `seek()` 根据 index entry 的 `last_key` 定位目标 block，再在 block 内二分。
  - block 读取继续复用 checksum、compression 和 block cache 逻辑。

- [x] **步骤 5：DB scan iterator 使用 SSTable 懒加载路径**

  修改 `DB::scan_iter_opt` 的 children 构造：

  - memtable 仍用 `EntryIterator`。
  - L0/lower-level tables 使用新的 SSTable scan iterator。
  - `ReadOptions.fill_cache` 继续传递到 SSTable iterator。
  - read sequence、range bound、tombstone/version 过滤继续由 `DBIterator` 负责。

- [x] **步骤 6：文档更新**

  更新：

  - `docs/implementation/README.md` 增加 V10。
  - `docs/usage.md` 增加 `scan_iter` 示例，并移除“scan 不是流式 iterator”的限制。
  - `README.md` 把当前限制改为“`scan` 仍返回 Vec，但可用 `scan_iter` 流式读取”。

- [x] **步骤 7：验证**

  运行：

  ```bash
  cargo test --test streaming_scan_iterator
  cargo test --all-targets
  cargo clippy --all-targets -- -D warnings
  cargo fmt --check
  git diff --check
  ```

  期望全部通过。

## 退出条件

- `DB::scan_iter` / `scan_iter_opt` 可作为公开 API 使用。
- `DB::scan` / `scan_opt` 保持原有 `Vec` 返回类型。
- SSTable scan iterator 不在构造时展开整张表。
- lower bound seek 能避免读取明显无关的前缀 blocks。
- `ReadOptions.fill_cache = false` 对 streaming scan 不产生 block cache hit/miss。
- snapshot、tombstone、旧版本过滤语义与旧 `scan` 一致。

## 建议提交

```bash
git add README.md docs src tests
git commit -m "feat: add V10 streaming scan iterator"
```
