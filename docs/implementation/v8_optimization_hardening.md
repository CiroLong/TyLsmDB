# V8 优化与加固实现计划

> **给智能代理执行者：** 必须使用子技能 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐任务执行本计划。步骤使用复选框（`- [ ]`）语法便于跟踪。

**目标：** 通过更好的 memtable 内部结构、压缩、group commit、subcompaction、metrics、benchmark、fuzzing 和崩溃测试提升性能与工程可信度。

**架构：** V8 保持公开 API 稳定，只在现有 trait 与模块边界后替换内部组件。加固测试通过可注入 Env 验证崩溃一致性，不依赖真实进程崩溃。

**技术栈：** Rust 2024、`crossbeam-skiplist`、`zstd`、`criterion`、`proptest`、`cargo test`、`cargo bench`。

---

## 范围

本版本包含：

- Skiplist memtable 与 arena allocation。
- Prefix compression 调优。
- 在 `CompressionType` 后接入 Snappy、LZ4 或 Zstd 压缩。
- Group commit。
- 大范围 compaction 的 subcompaction。
- Metrics 与 benchmark。
- Fault injection、model test、fuzz target。

本版本不包含：

- 不实现分布式复制。
- 不实现 SQL 层。
- 不实现 column family。
- 不实现 remote compaction。

## 文件划分

- 创建：`src/memtable/skiplist.rs`
- 创建：`src/memtable/arena.rs`
- 修改：`src/memtable/mod.rs`
- 修改：`src/table/block_builder.rs`
- 修改：`src/table/format.rs`
- 修改：`src/table/builder.rs`
- 修改：`src/table/reader.rs`
- 修改：`src/db.rs`
- 修改：`src/compact/executor.rs`
- 创建：`src/util/rate_limiter.rs`
- 创建：`src/metrics.rs`
- 创建：`benches/write_read.rs`
- 创建：`tests/model_kv.rs`
- 创建：`tests/crash_recovery.rs`
- 修改：`Cargo.toml`

## 任务

- [ ] **步骤 1：增加 benchmark 与测试依赖**

  加入 `Cargo.toml`：

  ```toml
  [dependencies]
  crossbeam-skiplist = "0.1"
  zstd = "0.13"

  [dev-dependencies]
  criterion = "0.5"
  proptest = "1.5"

  [[bench]]
  name = "write_read"
  harness = false
  ```

- [ ] **步骤 2：实现 memtable 策略**

  增加：

  ```rust
  pub enum MemTableKind {
      BTree,
      SkipList,
  }
  ```

  在 skiplist 测试和 benchmark 证明收益前，默认仍使用 BTree。`Options` 负责选择 kind。

- [ ] **步骤 3：增加 skiplist memtable**

  实现与 BTree memtable 相同的接口：

  - `put`
  - `delete`
  - `get`
  - `scan`
  - `approximate_size`

  两种 memtable kind 必须跑同一组行为测试。

- [ ] **步骤 4：增加 arena allocation**

  skiplist entry 的 internal key 和 value 存储到 arena 中。memtable drop 时释放整个 arena；不能暴露生命周期超过 memtable 的引用。

- [ ] **步骤 5：增加压缩**

  扩展 table block trailer 处理，支持：

  - `NoCompression`
  - `Zstd`

  Compression 由 `Options` 为每个 table 选择。Reader 对压缩后的 bytes 校验 checksum，再执行 decompression。

- [ ] **步骤 6：增加 group commit**

  把并发 writer 合并到一个 WAL append 与 sync group 中。保持每个 batch 的 sequence 顺序，并且只在 batch 已应用或 WAL error 已知后返回对应 writer。

- [ ] **步骤 7：增加 subcompaction**

  按不重叠 user-key range 切分大 compaction。每个 subcompaction 写独立 output file，父 task 最后应用一个合并后的 VersionEdit。

- [ ] **步骤 8：增加 rate limiter 与 metrics**

  跟踪：

  - 用户写入字节数。
  - WAL 写入字节数。
  - SST 写入字节数。
  - Compaction 读取与写入字节数。
  - Block cache hit/miss。
  - Bloom useful 与 full-positive 计数。
  - benchmark 输出中的 p50/p95/p99 写读延迟。

- [ ] **步骤 9：增加 model test**

  使用 `BTreeMap<Vec<u8>, Vec<u8>>` 作为 oracle。随机生成操作序列：

  - `put`
  - `delete`
  - `get`
  - `scan`
  - `flush`
  - `compact_range`
  - `reopen`

  每个操作后对比 TYLSMDB 可见结果与 oracle。

- [ ] **步骤 10：增加崩溃恢复测试**

  为文件操作增加可注入 `Env`，在以下位置注入失败：

  - WAL append half record。
  - WAL sync 前后。
  - SST 写半个文件。
  - SST sync 后、MANIFEST 前。
  - MANIFEST append half record。
  - CURRENT rename 前后。

  Reopen 后断言：synced writes 保留，unsynced writes 可以丢失，scan 仍有序，且不会出现重复 user key。

- [ ] **步骤 11：增加 benchmark**

  Benchmark：

  - Sequential write。
  - Random write。
  - Point lookup hit。
  - Point lookup miss。
  - Range scan。
  - WAL sync write latency。
  - Flush throughput。
  - Compaction throughput。

- [ ] **步骤 12：验证版本**

  运行：`cargo fmt`

  期望：命令成功退出。

  运行：`cargo test`

  期望：全部测试通过。

  运行：`cargo bench`

  期望：benchmark suite 完成并输出 latency/throughput metrics。

## 退出条件

- 公开 API 与 V7 保持兼容。
- Model test 与 crash test 覆盖设计文档中的持久化不变量。
- Compression、group commit、skiplist 行为都能通过 options 独立开关。
- Benchmark 能输出 write amplification、read amplification 和 latency 的可复现指标。

## 建议提交

```bash
git add Cargo.toml src benches tests
git commit -m "perf: harden TYLSMDB with optimized internals and tests"
```
