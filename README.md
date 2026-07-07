# TYLSMDB

TYLSMDB 是一个用 Rust 2024 编写的嵌入式 LSM tree key-value 存储引擎。它是一个 library crate，不是网络数据库服务；调用方通过 `DB` API 在本地目录中读写二进制 key/value。

当前实现已经完成 `docs/implementation/` 中 V0 到 V10 的主要路线：内存表、WAL recovery、SSTable flush、VersionSet/MANIFEST、读缓存、leveled compaction、MVCC snapshot/transaction、压缩、group commit、subcompaction、metrics、故障恢复测试、V9 的 API 语义与 `Env` 读写边界加固，以及 V10 的流式 scan iterator。

## 当前状态

按本仓库的 V0-V10 路线图，核心功能已经闭环，并有 integration tests、模型测试、fuzz smoke test、crash-recovery 故障注入测试和 Criterion benchmark 覆盖。

它仍然不是一个面向生产替代 RocksDB/LevelDB 的完整数据库产品。当前明确限制包括：

- `DB::scan` / `scan_opt` 仍返回 `Vec<(Vec<u8>, Vec<u8>)>` 作为便捷 API；大范围读取应使用 `scan_iter` / `scan_iter_opt`。
- flush 和 manual compaction 是同步 API；`Options.max_background_flushes`、`Options.max_background_compactions` 尚未启动后台 worker。
- `ReadOptions.verify_checksums` 当前不控制分支；SSTable block 解码会固定校验 checksum。
- `ReadOptions.total_order_seek` 当前没有行为分支。
- `Options.max_immutable_memtables` 与 `Options.bloom_false_positive_rate` 当前没有直接驱动主路径行为。
- 没有网络协议、认证授权、加密、column family、merge operator、SQL 层或 object-store Env。

更详细的 API 用法见 [docs/usage.md](docs/usage.md)，分阶段实现计划见 [docs/implementation/README.md](docs/implementation/README.md)，架构设计见 [docs/lsm_kv_design.md](docs/lsm_kv_design.md)。

## 功能概览

- **二进制 key/value API**：`put`、`get`、`delete`、`scan`、`scan_iter`、`WriteBatch`。
- **持久化写入路径**：WAL record 带 CRC32C，支持 partial/corrupt record recovery 语义，支持 `WriteOptions.sync` 和 `WalSyncMode::PerWrite`。
- **SSTable**：block prefix compression、Bloom filter、index/footer/properties、block checksum、可选 zstd block compression。
- **版本管理**：`VersionSet`、`MANIFEST-*`、`CURRENT`、file number、active WAL number、last sequence recovery。
- **读优化**：mutable/immutable memtable、L0、lower levels 的点查与范围 scan，streaming scan iterator，block cache、table cache、Bloom negative filter metrics。
- **Compaction**：leveled/manual compaction、tombstone/version GC、obsolete file cleanup、L0 写入压力处理、subcompaction 输出拆分。
- **MVCC 与事务**：snapshot read、水位线、乐观事务、read-your-own-writes、write/write、read/write 和 range phantom conflict 检测。
- **可配置存储边界**：`Options.env` 覆盖 WAL、MANIFEST、SSTable 的读写路径，测试可注入故障 Env。
- **观测与测试**：`metrics_snapshot`、`block_cache_stats`、`level_file_counts`，并提供模型测试、crash recovery 测试、fuzz harness 和 benchmark。

## 快速开始

本仓库当前没有发布到 crates.io。如果从另一个 Rust 项目使用，可以通过 path dependency 引入：

```toml
[dependencies]
tylsmdb = { path = "/path/to/tylsmdb" }
```

在本仓库内运行示例二进制：

```bash
cargo run
```

最小读写示例：

```rust
use tylsmdb::{DB, Options, Result};

fn main() -> Result<()> {
    let db = DB::open("target/example-db", Options::default())?;

    db.put(b"user:1", b"Alice")?;
    assert_eq!(db.get(b"user:1")?, Some(b"Alice".to_vec()));

    db.put(b"user:1", b"Alice v2")?;
    assert_eq!(db.get(b"user:1")?, Some(b"Alice v2".to_vec()));

    db.delete(b"user:1")?;
    assert_eq!(db.get(b"user:1")?, None);

    Ok(())
}
```

范围扫描：

```rust
use std::ops::Bound::{Excluded, Included};

use tylsmdb::{DB, Options, Result};

fn scan_example() -> Result<()> {
    let db = DB::open("target/scan-db", Options::default())?;
    db.put(b"a", b"1")?;
    db.put(b"b", b"2")?;
    db.put(b"c", b"3")?;

    let rows = db.scan(Included(b"a".as_slice()), Excluded(b"c".as_slice()))?;
    assert_eq!(
        rows,
        vec![(b"a".to_vec(), b"1".to_vec()), (b"b".to_vec(), b"2".to_vec())]
    );

    Ok(())
}
```

流式扫描：

```rust
use std::ops::Bound::Unbounded;

use tylsmdb::{DB, Options, Result};

fn scan_iter_example() -> Result<()> {
    let db = DB::open("target/scan-iter-db", Options::default())?;
    db.put(b"a", b"1")?;
    db.put(b"b", b"2")?;

    let mut iter = db.scan_iter(Unbounded, Unbounded)?;
    while iter.is_valid() {
        println!(
            "{}={}",
            String::from_utf8_lossy(iter.key().expect("valid key")),
            String::from_utf8_lossy(iter.value().expect("valid value"))
        );
        iter.next()?;
    }

    Ok(())
}
```

Snapshot 读：

```rust
use tylsmdb::{DB, Options, ReadOptions, Result};

fn snapshot_example() -> Result<()> {
    let db = DB::open("target/snapshot-db", Options::default())?;

    db.put(b"k", b"old")?;
    let snapshot = db.snapshot();
    db.put(b"k", b"new")?;

    let opts = ReadOptions {
        snapshot: Some(snapshot),
        ..ReadOptions::default()
    };
    assert_eq!(db.get_opt(b"k", opts)?, Some(b"old".to_vec()));

    Ok(())
}
```

事务：

```rust
use tylsmdb::{DB, Options, Result, TransactionOptions};

fn transaction_example() -> Result<()> {
    let db = DB::open("target/txn-db", Options::default())?;

    let mut txn = db.transaction(TransactionOptions::default())?;
    txn.put(b"a", b"1")?;
    txn.put(b"b", b"2")?;
    txn.commit()?;

    assert_eq!(db.get(b"a")?, Some(b"1".to_vec()));
    assert_eq!(db.get(b"b")?, Some(b"2".to_vec()));

    Ok(())
}
```

## 常用配置

```rust
use tylsmdb::memtable::MemTableKind;
use tylsmdb::table::format::CompressionType;
use tylsmdb::{Options, WalSyncMode};

let options = Options {
    wal_sync: WalSyncMode::PerWrite,
    memtable_kind: MemTableKind::SkipList,
    table_compression: CompressionType::Zstd,
    memtable_size: 256 * 1024,
    block_size: 8 * 1024,
    target_file_size_base: 4 * 1024 * 1024,
    max_subcompactions: 4,
    write_rate_limit_bytes_per_sec: Some(8 * 1024 * 1024),
    ..Options::default()
};
```

默认值要点：

| 配置 | 默认值 | 说明 |
| --- | --- | --- |
| `create_if_missing` | `true` | 目录不存在时创建 DB |
| `memtable_size` | `4 MiB` | mutable memtable 超过阈值后 freeze/flush |
| `wal_enabled` | `true` | 写入先追加 WAL |
| `wal_sync` | `Never` | 默认不为每次写入 fsync |
| `block_size` | `4 KiB` | SSTable data block 目标大小 |
| `block_cache_capacity` | `64 MiB` | block cache 近似容量 |
| `memtable_kind` | `BTree` | 可改为 `SkipList` |
| `table_compression` | `None` | 可改为 `Zstd` |
| `max_subcompactions` | `1` | compaction 输出拆分数量上限 |

## 数据文件

数据库文件存放在 `DB::open` 指定的目录下：

- `NNNNNN.wal`：WAL 文件。
- `NNNNNN.sst`：SSTable 文件。
- `MANIFEST-NNNNNN`：版本编辑日志。
- `CURRENT`：当前 MANIFEST 指针。

flush 和 compaction 会先写临时 SST 文件，再 rename 到最终文件名。WAL、MANIFEST 和 SSTable block 都有 checksum 或 magic value 校验。

## 构建、测试和基准

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
cargo test --test v7_mvcc_transactions
cargo test --test v8_optimization_hardening
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
tests/                  V0-V10、模型、fuzz、crash recovery 测试
docs/                   设计、使用方式和分版本实现计划
```

## 设计约束

- key 和 value 是任意 bytes，不要求 UTF-8。
- `InternalKey` 排序语义必须保持：user key 升序、sequence 降序、value type 兜底排序。
- DB 行为相关文件系统 mutation/read 应通过 `Env`，便于故障注入测试覆盖。
- 公开 API 从 `src/lib.rs` re-export；内部 fallible 函数使用 `crate::error::{Error, Result}`。
- 存储损坏映射为 `Error::Corruption`，参数错误映射为 `Error::InvalidArgument`，关闭后的访问映射为 `Error::Closed`，事务冲突映射为 `Error::TransactionConflict`。

## 文档

- [API 使用指南](docs/usage.md)
- [LSM KV 架构设计](docs/lsm_kv_design.md)
- [实现路线图](docs/implementation/README.md)
- [V9 API 语义与 Env 边界加固](docs/implementation/v9_api_semantics_hardening.md)
- [V10 流式 Scan Iterator](docs/implementation/v10_streaming_scan_iterator.md)
