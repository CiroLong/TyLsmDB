# TYLSMDB

TYLSMDB 是一个用 Rust 2024 编写的嵌入式 LSM tree key-value 存储引擎。它是 library crate，不是网络数据库服务；调用方通过 `DB` API 在本地目录中读写二进制 key/value。

## 当前能力

当前代码已经具备一条完整的单机嵌入式 KV 存储主线：

- 二进制 key/value API：`put`、`get`、`delete`、`scan`、`scan_iter`、`WriteBatch`。
- 持久化写入路径：WAL、CRC32C、显式 sync、per-write sync、group commit、reopen recovery。
- SSTable：block prefix compression、Bloom filter、properties、index/footer、block checksum、可选 zstd block compression。
- 版本管理：`VersionSet`、`MANIFEST-*`、`CURRENT`、file number、active WAL number、last sequence recovery。
- 读路径：mutable/immutable memtable、L0、lower levels、streaming scan iterator、block cache、table cache。
- Compaction：leveled/manual compaction、tombstone/version GC、obsolete file cleanup、L0 写入压力处理、subcompaction 输出拆分。
- MVCC 与事务：snapshot read、水位线、乐观事务、read-your-own-writes、write/write、read/write 和 range phantom conflict 检测。
- 可测试存储边界：`Options.env` 覆盖 WAL、MANIFEST、SSTable 的读写路径，支持故障注入测试。
- 观测与验证：`metrics_snapshot`、`block_cache_stats`、`level_file_counts`、模型测试、fuzz smoke test、crash recovery 测试和 Criterion benchmark。

当前明确限制：

- `DB::scan` / `scan_opt` 返回 `Vec<(Vec<u8>, Vec<u8>)>`；大范围读取应使用 `scan_iter` / `scan_iter_opt`。
- flush 和 manual compaction 是同步 API；`Options.max_background_flushes`、`Options.max_background_compactions` 尚未启动后台 worker。
- `ReadOptions.verify_checksums` 和 `ReadOptions.total_order_seek` 当前没有行为分支。
- `Options.max_immutable_memtables` 与 `Options.bloom_false_positive_rate` 当前没有直接驱动主路径行为。
- 没有网络协议、认证授权、加密、column family、merge operator、SQL 层或 object-store Env。

## 快速开始

本仓库当前没有发布到 crates.io。如果从另一个 Rust 项目使用，可以通过 path dependency 引入：

```toml
[dependencies]
tylsmdb = { path = "/path/to/tylsmdb" }
```

最小读写示例：

```rust
use tylsmdb::{DB, Options, Result};

fn main() -> Result<()> {
    let db = DB::open("target/example-db", Options::default())?;

    db.put(b"user:1", b"Alice")?;
    assert_eq!(db.get(b"user:1")?, Some(b"Alice".to_vec()));

    db.delete(b"user:1")?;
    assert_eq!(db.get(b"user:1")?, None);

    Ok(())
}
```

更多使用方式见 [docs/usage.md](docs/usage.md)。

## 构建与测试

```bash
cargo build
cargo test
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

常用专项测试：

```bash
cargo test --test model_kv
cargo test --test crash_recovery
cargo test --test mvcc_transactions
cargo test --test optimization_hardening
```

fuzz/model harness：

```bash
cargo test --test fuzz_target
cargo run --bin fuzz_model < input.bin
```

benchmark：

```bash
cargo bench --bench write_read
```

测试、示例和 benchmark 数据会写入 `target/` 下的路径，例如 `target/tylsmdb-tests`、`target/tylsmdb-example` 和 `target/tylsmdb-benches`。

## 仓库结构

```text
src/db.rs               DB 编排、读写、flush、compaction、snapshot、metrics
src/memtable/           BTree/SkipList memtable 与 arena
src/key/                InternalKey 与排序语义
src/wal/                WAL reader/writer 与 record 格式
src/table/              SSTable block、filter、builder、reader
src/version/            VersionSet、VersionEdit、MANIFEST、CURRENT
src/iterator/           storage iterator、merge iterator、DBIterator
src/cache/              block cache 与 table cache
src/compact/            compaction picker、task、executor
src/mvcc/               snapshot watermark 与冲突检测
src/env/                文件系统抽象与文件名工具
tests/                  按功能模块组织的 integration、模型、fuzz、crash recovery 测试
docs/                   目标路线和 API 使用说明
```

## 设计约束

- key 和 value 是任意 bytes，不要求 UTF-8。
- `InternalKey` 排序语义必须保持：user key 升序、sequence 降序、value type 兜底排序。
- DB 行为相关文件系统 mutation/read 应通过 `Env`，便于故障注入测试覆盖。
- 公开 API 从 `src/lib.rs` re-export；内部 fallible 函数使用 `crate::error::{Error, Result}`。
- 存储损坏映射为 `Error::Corruption`，参数错误映射为 `Error::InvalidArgument`，关闭后的访问映射为 `Error::Closed`，事务冲突映射为 `Error::TransactionConflict`。

## 文档

- [目标路线](docs/roadmap.md)
- [API 使用说明](docs/usage.md)
