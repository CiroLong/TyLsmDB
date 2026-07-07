# TYLSMDB 架构说明

本文面向需要快速理解仓库结构的 agent 和贡献者，内容只总结当前 checkout 中实际存在的信息。仓库里没有已有的 `AGENTS.md`、`AGENT.md`、`.cursor/rules/`、`.github/copilot-instructions.md` 或 `.trae/rules/` 文件需要合并。

## 项目概览

`tylsmdb` 是一个基于 LSM tree 的 Rust 2024 嵌入式 key-value 存储引擎。它是 library crate，`src/main.rs` 里有一个很小的示例二进制；它不是网络数据库服务。公开 API 从 `src/lib.rs` 导出，核心类型包括 `DB`、`Options`、`ReadOptions`、`WriteOptions`、`WriteBatch`、`Snapshot`、`Transaction` 和 `MetricsSnapshot`。

设计文档在 `docs/` 下。`docs/lsm_kv_design.md` 描述目标中的 LevelDB/RocksDB 风格架构，`docs/implementation/README.md` 记录 V0 到 V8 的实现里程碑。当前代码已经包含 V8 阶段的主要能力：WAL recovery、SSTable flush、VersionSet/MANIFEST、带 cache 的读路径、leveled compaction、MVCC snapshot/transaction、compression、skiplist memtable、group commit、subcompaction、metrics，以及 crash-recovery 加固测试。

高层模块分工：

- `src/db.rs` 负责 open/recovery、read、write、flush、compaction、snapshot、transaction、write grouping、rate limiting、metrics，以及 table/cache 状态编排。
- `src/memtable/` 通过 `MemTable` enum 提供可变内存表，底层可选 `BTreeMap` 或 `crossbeam_skiplist`，skiplist 路径配有简单 arena。
- `src/key/` 定义 `InternalKey = user key + sequence + value type`；排序规则是 user key 升序、sequence 降序、value type 兜底排序，因此同一个 user key 的最新可见版本排在最前。
- `src/wal/` 存储带 CRC32C checksum 的 full WAL record。完整但损坏的 record 会返回错误；末尾 partial record 在恢复时按 EOF 处理。
- `src/table/` 实现 SSTable block、block prefix compression、Bloom filter、table properties、index/footer 编码、可选 zstd block compression，以及 block checksum 校验。
- `src/version/` 维护 `Version`、`VersionSet`、`VersionEdit`、`MANIFEST-*`、`CURRENT`、file number、active WAL number 和持久化的 last sequence。
- `src/iterator/` 合并不同 storage source，并在 scan 时过滤旧版本和 tombstone。
- `src/cache/` 包含读路径使用的 block cache 和 table cache。
- `src/compact/` 负责选择并执行 leveled/manual compaction，包括 tombstone/version GC 和 subcompaction 输出拆分。
- `src/mvcc/`、`src/snapshot.rs` 和 `src/transaction.rs` 实现 snapshot、watermark、乐观事务、读写冲突检查和 range phantom 检测。
- `src/env/` 抽象文件系统操作；测试通过 `Options.env` 注入故障版 env。

关键数据流：

- 写入以 `WriteBatch` 进入 DB。DB 分配连续 sequence number，把编码后的 batch 追加到 WAL（除非本次写入禁用 WAL），按需 sync WAL，然后把记录写入 mutable memtable；当 `Options.memtable_size` 超限时冻结并触发 flush。
- Flush 会把最老的可 flush memtable 写到临时 SST 路径，完成并 sync table 后 rename 成 `NNNNNN.sst`，再向 MANIFEST 记录 L0 `AddFile` 和 `LastSequence`，打开新的 WAL number，并替换 active WAL。
- 点查先查 mutable memtable，再按新到旧查 immutable memtables、L0 tables，最后查 lower levels。SSTable 读路径使用 Bloom filter 和 block cache。
- Scan 会为 memtable 和 SSTable entries 构建 iterator，合并后由 `DBIterator` 按请求边界和 read sequence 返回有序、可见的 user key/value。
- Compaction 读取选中的 input files，基于 active snapshot watermark 删除过期版本，仅在更低层没有 key range 重叠时删除 tombstone，写出新的 SSTable，向 MANIFEST 记录 file delete/add，刷新 table state，并删除 obsolete input files。

## 构建与命令

本 crate 只使用 Cargo；仓库中没有 Makefile、Dockerfile、部署配置或自定义 formatter 配置。

- `cargo build` 构建 library 和 binaries。
- `cargo run` 运行示例二进制，它会打开 `target/tylsmdb-example`。
- `cargo test` 运行 unit tests 和 integration tests。
- `cargo test --test model_kv` 运行基于 `BTreeMap` oracle/proptest 的模型测试。
- `cargo test --test crash_recovery` 运行故障注入 crash/recovery 测试。
- `cargo test --test fuzz_target` 构建并运行 `fuzz_model` binary 的 smoke test。
- `cargo run --bin fuzz_model < input.bin` 把 operation bytes 输入模型检查 binary。
- `cargo bench --bench write_read` 运行 Criterion benchmarks，并把 amplification 输出写到 `target/tylsmdb-benches/`。
- `cargo fmt` 应用标准 rustfmt 格式化。
- `cargo clippy --all-targets` 是这个 Cargo crate 的自然 lint 命令；仓库中没有额外 clippy 配置。

测试、benchmark、fuzz 和示例数据会写入 `target/` 下的路径，例如 `target/tylsmdb-tests`、`target/tylsmdb-benches`、`target/tylsmdb-fuzz` 和 `target/tylsmdb-example`。

仓库里没有部署命令。应把 `tylsmdb` 视为供其他 Rust 应用嵌入使用的 library crate。

## 代码风格

代码使用 Rust 2024 edition 和普通 rustfmt 风格。仓库中没有 `rustfmt.toml` 或 `clippy.toml`。

项目内约定：

- 公开 API 从 `src/lib.rs` re-export；只有确实面向 crate 用户的类型才应新增到这里。
- fallible 的内部/公开函数使用 `crate::error::{Error, Result}`。存储损坏通常映射为 `Error::Corruption`，调用参数错误映射为 `Error::InvalidArgument`，关闭后的 DB 访问映射为 `Error::Closed`，事务冲突映射为 `Error::TransactionConflict`。
- user key 和 value 都是 binary data。`src/bytes.rs` 中的本地别名是 `Bytes = Vec<u8>`、`UserKey = [u8]`、`UserKeyBuf = Vec<u8>`。
- 改读路径、table、compaction 或 memtable 行为时，必须保持 `InternalKey` 排序语义：user key 升序、sequence 降序、value type 兜底排序。
- 记录编码保持显式，并在已有模式下通过 tag/record type 做版本区分：WAL record header、`WriteBatch` payload、SSTable footer/index/properties、`VersionEdit` tag 都会拒绝 trailing data 或 unknown data。
- 属于 DB 行为的文件系统 mutation 应通过 `Env` 进行，这样 crash-recovery/fault-injection 测试才能观察或注入失败。
- flush/compaction 输出沿用临时 SST 文件加 rename 的方式，和 `DB::write_l0_table` 及 compaction output 代码保持一致。
- 注意 `DB` 中的锁关系：state 在 `RwLock` 后面，versions/WAL/write group 各自有 `Mutex`，lock poisoning 会转换成 `Error::Corruption`。
- 不要假设 key 是字符串。测试为了可读性经常使用类似 UTF-8 的 key，但存储 API 接受的是 `&[u8]`。
- 测试使用 `target/tylsmdb-tests/<name>` 下的 fresh directory，打开 DB 前会删除旧内容。

依赖刻意保持较少：`crc32c`、`crossbeam-skiplist`、`zstd`，以及 dev dependencies `criterion` 和 `proptest`。

## 测试

测试主要由 integration tests 驱动，部分模块内也有聚焦的 unit tests。

integration tests 按功能里程碑组织：

- `tests/v0_v1.rs`：公开 API 形状、internal key ordering、内存态 put/get/delete/scan 行为。
- `tests/v2_wal_recovery.rs`：varint/batch 编码、WAL reader/writer、partial/corrupt WAL 处理、reopen recovery。
- `tests/v3_sstable_flush.rs`：block prefix compression、SSTable roundtrip/checksum、memtable flush、scan merging。
- `tests/v4_versions_manifest.rs`：MANIFEST/CURRENT 处理、version replay、file-number preservation、跨 SST/WAL recovery。
- `tests/v5_read_path_cache.rs`：DB iterator visibility、L0/lower-level read 行为、Bloom filter、block cache stats。
- `tests/v6_leveled_compaction_gc.rs`：compaction picking/execution、level file counts、GC 行为。
- `tests/v7_mvcc_transactions.rs`：snapshot、基于 watermark 的 GC、原子 transaction commit、rollback、write/write 和 read/write conflict、phantom range conflict。
- `tests/v8_optimization_hardening.rs`：skiplist memtable parity、zstd table compression、metrics、group commit、rate limiter、subcompaction。
- `tests/model_kv.rs`：deterministic 和 proptest operation streams，对照 `BTreeMap` oracle。
- `tests/crash_recovery.rs`：通过自定义 `Env` 注入 WAL/SST/MANIFEST/CURRENT 故障。
- `tests/fuzz_target.rs` 和 `src/bin/fuzz_model.rs`：基于紧凑 operation bytes 的 binary fuzz/model harness。

常用聚焦命令：

- `cargo test put_get_and_delete_work_in_memory`
- `cargo test --test v7_mvcc_transactions`
- `cargo test --test crash_recovery`
- `cargo test --test model_kv random_operations_match_btreemap_oracle`
- `cargo bench --bench write_read`

修改 persistence 或 recovery 时，至少运行 WAL、SSTable、manifest、model 和 crash-recovery 相关测试。修改 visibility/sequence 逻辑时，包含 MVCC transaction tests 和 model tests。修改 read path caching 时，包含 `v5_read_path_cache` 和 `v8_optimization_hardening`。

## 安全与数据保护

这个 crate 没有认证、授权、加密、网络协议或 secret-management 层。它只在 `DB::open` 传入的 DB path 下存储本地文件。

当前已有的存储完整性机制：

- WAL 和 MANIFEST record 包含基于 record type 加 payload 的 CRC32C。
- SSTable block 包含 compression type 和 CRC32C trailer；reader 在解码前校验 block。
- SSTable 带有固定 footer magic value。
- decoder 会拒绝 unknown record type/tag，以及结构化 payload 末尾的 trailing bytes。
- recovery 会忽略末尾 partial WAL/MANIFEST record，但对 checksum 非法的完整 record 返回 corruption error。
- flush 和 compaction 先写临时 SST 文件，完成后 rename 到最终文件名。

数据持久性由选项控制：

- `Options.wal_enabled` 默认是 `true`。
- `Options.wal_sync` 默认是 `WalSyncMode::Never`；如果测试或调用方要求写入返回前完成 fsync，使用 `WalSyncMode::PerWrite` 或 `WriteOptions { sync: true, .. }`。
- `WriteOptions.disable_wal` 会在 WAL 全局启用时跳过本次写入的 WAL；除非后续 flush 并通过 manifest 路径持久化，否则这类写入在 crash 后可能丢失。
- `Options.table_compression` 默认不压缩，可以设置为 zstd。

key 和 value 是任意 bytes；除非测试需要，不要新增会打印原始 user data 的日志。现有 conflict message 使用 `String::from_utf8_lossy` 格式化 key，这对测试有用，但不是保密边界。

## 配置

配置通过 Rust struct 完成，主要是 `Options`、`ReadOptions`、`WriteOptions` 和 `TransactionOptions`。仓库里没有环境变量或外部配置文件。

重要 `Options` 字段和默认值：

- 打开行为：`create_if_missing = true`，`error_if_exists = false`。
- 内存/写缓冲：`memtable_size = 4 MiB`，`max_immutable_memtables = 3`，`memtable_kind = MemTableKind::BTree`。
- Table layout：`block_size = 4 KiB`，`target_file_size_base = 64 MiB`，`table_compression = CompressionType::None`。
- Levels/compaction pressure：`max_levels = 7`，L0 compaction trigger `4`，slowdown trigger `12`，stop trigger `20`，base level size `256 MiB`，multiplier `10.0`。
- WAL/durability：`wal_enabled = true`，`wal_sync = WalSyncMode::Never`，`write_group_max_delay = 250us`。
- 读优化：Bloom false-positive rate `0.01`，block cache capacity `64 MiB`。
- background/subcompaction 相关旋钮已经在 options 中存在，但当前代码通过同步 DB 方法执行 flush 和 compaction。
- 写限流：`write_rate_limit_bytes_per_sec: Option<u64>`。
- 文件系统：`env: Arc<dyn Env>` 默认是 `FsEnv`；测试会替换它来注入失败。

`ReadOptions` 默认启用 checksum verification、启用 cache fill、关闭 total-order seek，并且不带 snapshot。当前 SSTable block 读取会在解码过程中校验 checksum。

`TransactionOptions` 当前只有 `read_only`；read-only transaction 会拒绝 `put` 和 `delete`。

数据库文件名由 `src/env/file.rs` 派生：`NNNNNN.wal`、`NNNNNN.sst`、`MANIFEST-NNNNNN` 和 `CURRENT`。
