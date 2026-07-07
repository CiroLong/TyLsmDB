# TYLSMDB API 使用指南

本文说明当前源码中已经实现的 API 用法。示例按 crate 用户视角编写，未使用设计文档中尚未公开或未实现的接口。

## 导入

常用 API 已在 crate 根导出：

```rust
use tylsmdb::{
    DB, Error, Options, ReadOptions, Result, TransactionOptions, WalSyncMode,
    WriteBatch, WriteOptions,
};
```

少数配置类型需要从子模块导入：

```rust
use tylsmdb::memtable::MemTableKind;
use tylsmdb::table::format::CompressionType;
```

所有 key/value 都按 bytes 处理。`put`、`delete`、`get`、`scan` 等 API 接收 `&[u8]`，读取返回 `Vec<u8>`。

## 打开数据库

```rust
use tylsmdb::{DB, Options, Result};

fn main() -> Result<()> {
    let db = DB::open("target/my-db", Options::default())?;
    println!("opened at {}", db.path().display());
    Ok(())
}
```

`Options::default()` 会在路径不存在时创建数据库。打开已有路径时，如果目录下有 `CURRENT`，会通过 `MANIFEST-*` 和当前 WAL 恢复状态；否则创建新的 `VersionSet`、`CURRENT` 和 MANIFEST。

打开相关选项：

```rust
use tylsmdb::{DB, Options, Result};

fn open_existing_only() -> Result<DB> {
    DB::open(
        "target/existing-db",
        Options {
            create_if_missing: false,
            ..Options::default()
        },
    )
}
```

- `create_if_missing = true`：默认值，目录不存在时创建。
- `error_if_exists = true`：目标路径已经存在时返回 `Error::InvalidArgument`。
- `create_if_missing = false`：目标路径不存在时返回 `Error::InvalidArgument`。

`DB` 是可 clone 的轻量句柄，内部状态通过 `Arc` 共享。

## 基础读写

```rust
use tylsmdb::{DB, Options, Result};

fn basic_kv() -> Result<()> {
    let db = DB::open("target/basic-kv", Options::default())?;

    db.put(b"user:1", b"Alice")?;
    assert_eq!(db.get(b"user:1")?, Some(b"Alice".to_vec()));

    db.put(b"user:1", b"Alice v2")?;
    assert_eq!(db.get(b"user:1")?, Some(b"Alice v2".to_vec()));

    db.delete(b"user:1")?;
    assert_eq!(db.get(b"user:1")?, None);

    Ok(())
}
```

`put` 写入或覆盖 key，`delete` 写入 tombstone，`get` 返回最新可见值。空 key、二进制 key、二进制 value 都可以使用，不要求 UTF-8。

## 批量写入

`WriteBatch` 是一次原子写入单位。DB 会为 batch 内记录分配连续 sequence number，并按 batch 顺序应用：

```rust
use tylsmdb::{DB, Options, Result, WriteBatch, WriteOptions};

fn write_batch() -> Result<()> {
    let db = DB::open("target/batch-db", Options::default())?;

    let mut batch = WriteBatch::new();
    batch.put(b"a".to_vec(), b"1".to_vec());
    batch.put(b"b".to_vec(), b"2".to_vec());
    batch.delete(b"a".to_vec());

    db.write(batch, WriteOptions::default())?;

    assert_eq!(db.get(b"a")?, None);
    assert_eq!(db.get(b"b")?, Some(b"2".to_vec()));
    Ok(())
}
```

`WriteOptions`：

```rust
use tylsmdb::WriteOptions;

let opts = WriteOptions {
    sync: true,
    disable_wal: false,
};
```

- `sync = true`：本次写入要求 WAL append 后 sync。
- `disable_wal = true`：全局 WAL 启用时跳过本次写入的 WAL。除非之后 flush 并通过 MANIFEST 持久化，否则 crash 后可能丢失。
- 空 batch 调用 `DB::write` 会直接返回 `Ok(())`。

## WAL 与持久性

默认 WAL 配置：

```rust
use tylsmdb::{Options, WalSyncMode};

let options = Options {
    wal_enabled: true,
    wal_sync: WalSyncMode::Never,
    ..Options::default()
};
```

持久性相关行为：

- `wal_enabled = true`：写入先追加到 WAL，再进入 memtable。
- `wal_sync = WalSyncMode::Never`：默认值，写入不会每次自动 fsync。
- `wal_sync = WalSyncMode::PerWrite`：每次写入都会 WAL sync，并且 group commit 会等待 `write_group_max_delay` 聚合并发写。
- `WriteOptions { sync: true, .. }`：只要求本次写入 sync。
- `DB::sync_wal()`：显式 sync 当前 WAL。

示例：

```rust
use tylsmdb::{DB, Options, Result, WalSyncMode};

fn durable_writes() -> Result<()> {
    let db = DB::open(
        "target/durable-db",
        Options {
            wal_sync: WalSyncMode::PerWrite,
            ..Options::default()
        },
    )?;

    db.put(b"k", b"v")?;
    Ok(())
}
```

## 范围扫描

`scan` 返回有序、去重后的可见 user key/value 列表。边界使用 `std::ops::Bound<&[u8]>`：

```rust
use std::ops::Bound::{Excluded, Included, Unbounded};

use tylsmdb::{DB, Options, Result};

fn scan_examples() -> Result<()> {
    let db = DB::open("target/scan-db", Options::default())?;
    db.put(b"a", b"1")?;
    db.put(b"b", b"2")?;
    db.put(b"c", b"3")?;

    let rows = db.scan(Included(b"a".as_slice()), Excluded(b"c".as_slice()))?;
    assert_eq!(
        rows,
        vec![
            (b"a".to_vec(), b"1".to_vec()),
            (b"b".to_vec(), b"2".to_vec()),
        ]
    );

    let all = db.scan(Unbounded, Unbounded)?;
    assert_eq!(all.len(), 3);

    Ok(())
}
```

`scan` 会合并 mutable memtable、immutable memtables、L0 tables 和 lower levels，并按 read sequence 过滤旧版本和 tombstone。

## Snapshot 读

`DB::snapshot()` 保存创建时的 `read_seq`。通过 `ReadOptions { snapshot: Some(snapshot), .. }` 可以做一致性读：

```rust
use std::ops::Bound::Unbounded;

use tylsmdb::{DB, Options, ReadOptions, Result};

fn snapshot_read() -> Result<()> {
    let db = DB::open("target/snapshot-db", Options::default())?;

    db.put(b"k", b"old")?;
    let snapshot = db.snapshot();
    db.put(b"k", b"new")?;

    assert_eq!(db.get(b"k")?, Some(b"new".to_vec()));

    assert_eq!(
        db.get_opt(
            b"k",
            ReadOptions {
                snapshot: Some(snapshot.clone()),
                ..ReadOptions::default()
            },
        )?,
        Some(b"old".to_vec())
    );

    let rows = db.scan_opt(
        Unbounded,
        Unbounded,
        ReadOptions {
            snapshot: Some(snapshot),
            ..ReadOptions::default()
        },
    )?;
    assert_eq!(rows, vec![(b"k".to_vec(), b"old".to_vec())]);

    Ok(())
}
```

活跃 snapshot 会影响 compaction GC：低于活跃 snapshot 仍可能可见的旧版本不会被删除。`Snapshot::clone()` 会登记同一个 read sequence，drop 会释放 watermark 引用。

`ReadOptions` 字段：

- `snapshot`：指定读序列；不传时读取 DB 当前最新 sequence。
- `fill_cache`：控制 SSTable block 读取时是否使用/填充 block cache。
- `verify_checksums`：当前读路径没有使用该字段；SSTable block 解码时会固定校验 checksum。
- `total_order_seek`：当前实现没有使用该字段。

## 事务

通过 `DB::transaction(TransactionOptions)` 创建事务。事务持有创建时的 snapshot，支持 read-your-own-writes，并在 commit 时做乐观冲突检查。

```rust
use tylsmdb::{DB, Options, Result, TransactionOptions};

fn transaction_commit() -> Result<()> {
    let db = DB::open("target/txn-db", Options::default())?;

    let mut txn = db.transaction(TransactionOptions::default())?;
    txn.put(b"a", b"1")?;
    txn.put(b"b", b"2")?;

    assert_eq!(txn.get(b"a")?, Some(b"1".to_vec()));
    assert_eq!(db.get(b"a")?, None);

    txn.commit()?;

    assert_eq!(db.get(b"a")?, Some(b"1".to_vec()));
    assert_eq!(db.get(b"b")?, Some(b"2".to_vec()));
    Ok(())
}
```

Rollback 会丢弃事务内写入：

```rust
use tylsmdb::{DB, Options, Result, TransactionOptions};

fn transaction_rollback() -> Result<()> {
    let db = DB::open("target/txn-rollback-db", Options::default())?;
    let mut txn = db.transaction(TransactionOptions::default())?;

    txn.put(b"k", b"value")?;
    txn.rollback()?;

    assert_eq!(db.get(b"k")?, None);
    Ok(())
}
```

只读事务：

```rust
use tylsmdb::{DB, Options, Result, TransactionOptions};

fn read_only_transaction() -> Result<()> {
    let db = DB::open("target/read-only-txn-db", Options::default())?;
    let mut txn = db.transaction(TransactionOptions { read_only: true })?;

    let _ = txn.get(b"k")?;
    assert!(txn.put(b"k", b"v").is_err());

    txn.rollback()
}
```

冲突语义：

- 事务读过的 key 在事务 snapshot 之后被其他写入修改，`commit` 返回 `Error::TransactionConflict`。
- 事务准备写入的 key 在事务 snapshot 之后被其他写入修改，`commit` 返回 `Error::TransactionConflict`。
- 事务 scan 过的 range 在事务 snapshot 之后发生插入、删除或更新，`commit` 返回 `Error::TransactionConflict`。
- `commit(self)` 和 `rollback(self)` 会消费事务对象。

冲突处理示例：

```rust
use tylsmdb::{DB, Error, Options, Result, TransactionOptions};

fn handle_conflict() -> Result<()> {
    let db = DB::open("target/conflict-db", Options::default())?;
    db.put(b"k", b"base")?;

    let mut txn = db.transaction(TransactionOptions::default())?;
    assert_eq!(txn.get(b"k")?, Some(b"base".to_vec()));

    db.put(b"k", b"outside")?;
    txn.put(b"other", b"value")?;

    match txn.commit() {
        Err(Error::TransactionConflict(msg)) => {
            println!("retry transaction: {msg}");
        }
        other => other?,
    }

    Ok(())
}
```

## Flush 和 compaction

`flush` 把当前 mutable memtable 或最老 immutable memtable 写成 L0 SSTable。如果没有可 flush 的数据，它返回 `Ok(())`。

```rust
use tylsmdb::{DB, Options, Result};

fn flush_now() -> Result<()> {
    let db = DB::open("target/flush-db", Options::default())?;
    db.put(b"k", b"v")?;
    db.flush()?;
    Ok(())
}
```

`compact_range` 对指定 key range 执行 manual compaction：

```rust
use std::ops::Bound::{Included, Unbounded};

use tylsmdb::{DB, Options, Result};

fn compact_range() -> Result<()> {
    let db = DB::open("target/compact-db", Options::default())?;

    db.put(b"a", b"1")?;
    db.flush()?;
    db.put(b"a", b"2")?;
    db.flush()?;

    db.compact_range(Included(b"a".as_slice()), Unbounded)?;
    Ok(())
}
```

Compaction 会根据活跃 snapshot watermark 决定哪些旧版本可以删除。仍需要旧版本读取的 snapshot 存活时，不要期待 compaction 删除这些旧版本。

## 配置示例

选择 skiplist memtable 和 zstd table compression：

```rust
use tylsmdb::memtable::MemTableKind;
use tylsmdb::table::format::CompressionType;
use tylsmdb::{DB, Options, Result};

fn tuned_open() -> Result<DB> {
    DB::open(
        "target/tuned-db",
        Options {
            memtable_kind: MemTableKind::SkipList,
            table_compression: CompressionType::Zstd,
            memtable_size: 256 * 1024,
            block_size: 8 * 1024,
            target_file_size_base: 4 * 1024 * 1024,
            max_subcompactions: 4,
            ..Options::default()
        },
    )
}
```

写入限流和 L0 写入压力配置：

```rust
use tylsmdb::{DB, Options, Result};

fn throttled_open() -> Result<DB> {
    DB::open(
        "target/throttled-db",
        Options {
            write_rate_limit_bytes_per_sec: Some(4 * 1024 * 1024),
            level0_slowdown_writes_trigger: 8,
            level0_stop_writes_trigger: 16,
            ..Options::default()
        },
    )
}
```

当前会影响行为的常用 `Options`：

- `memtable_size`：mutable memtable 近似大小达到阈值后触发 freeze/flush。
- `block_size`：SSTable data block 目标大小。
- `target_file_size_base`：compaction 输出拆分目标大小。
- `max_levels`：Version 的 level 数量。
- `level0_file_num_compaction_trigger`：compaction picker 计算 L0 score 使用。
- `level0_slowdown_writes_trigger`：L0 文件数达到阈值后写入会短暂 sleep。
- `level0_stop_writes_trigger`：L0 文件数达到阈值后写入前会先尝试 compact all range。
- `max_bytes_for_level_base` / `max_bytes_for_level_multiplier`：leveled compaction score 使用。
- `wal_enabled` / `wal_sync` / `write_group_max_delay`：控制 WAL 和 group commit。
- `block_cache_capacity`：控制 block cache 近似容量；当前实现按 `capacity_bytes / 4096` 估算最大 block 数。
- `memtable_kind`：选择 `BTree` 或 `SkipList` memtable。
- `table_compression`：选择 `CompressionType::None` 或 `CompressionType::Zstd`。
- `write_rate_limit_bytes_per_sec`：写入前按用户 key/value 字节数限流。
- `max_subcompactions`：compaction 输出可拆分的 chunk 数量上限。
- `env`：文件系统抽象，测试可注入故障 env。

当前存在但没有直接影响主路径行为的字段：

- `max_immutable_memtables`
- `max_background_flushes`
- `max_background_compactions`
- `bloom_false_positive_rate`，当前 Bloom filter builder 没有读取该配置。

## 观测指标

```rust
use tylsmdb::{DB, Options, Result};

fn inspect_metrics() -> Result<()> {
    let db = DB::open("target/metrics-db", Options::default())?;
    db.put(b"k", b"value")?;
    db.flush()?;
    let _ = db.get(b"k")?;

    let metrics = db.metrics_snapshot();
    println!("user_write_bytes={}", metrics.user_write_bytes);
    println!("wal_write_bytes={}", metrics.wal_write_bytes);
    println!("sst_write_bytes={}", metrics.sst_write_bytes);
    println!("block_cache_hits={}", metrics.block_cache_hits);
    println!("block_cache_misses={}", metrics.block_cache_misses);

    Ok(())
}
```

可用观测 API：

- `DB::metrics_snapshot() -> MetricsSnapshot`
- `DB::block_cache_stats() -> CacheStats`
- `DB::level_file_counts() -> Vec<usize>`

`level_file_counts` 返回从 L0 开始的每层文件数。读取失败时当前实现返回空 `Vec`。

## 关闭和错误处理

`DB::close()` 只把 DB 标记为 closed。之后通过该句柄执行读写、flush、transaction 等会返回 `Error::Closed`。drop `DB` 句柄本身没有额外公开关闭流程。

```rust
use tylsmdb::{DB, Error, Options, Result};

fn close_example() -> Result<()> {
    let db = DB::open("target/close-db", Options::default())?;
    db.close()?;

    assert!(matches!(db.get(b"k"), Err(Error::Closed)));
    Ok(())
}
```

公开错误类型：

- `Error::InvalidArgument(String)`：参数或打开条件不满足。
- `Error::Corruption(String)`：WAL、MANIFEST、SSTable、锁 poisoning 等损坏或不一致情况。
- `Error::Io(std::io::Error)`：文件系统错误。
- `Error::Closed`：DB 或 transaction 已关闭。
- `Error::Unsupported(&'static str)`：当前源码定义了该 variant，主路径很少使用。
- `Error::TransactionConflict(String)`：事务 commit 冲突。

## 当前 API 注意事项

- `DB::scan` 和 `scan_opt` 当前直接返回 `Vec<(Vec<u8>, Vec<u8>)>`，不是流式 iterator。
- `ReadOptions.verify_checksums` 和 `ReadOptions.total_order_seek` 当前没有实际控制分支。
- `Options.max_background_flushes` 和 `Options.max_background_compactions` 当前没有启动后台 worker；flush 和 manual compaction 是同步方法。
- `WriteBatch::encode_with_sequence` 和 `decode_payload` 是公开方法，但主要供 WAL/recovery 路径使用；普通用户通常只需要 `new`、`put`、`delete` 和 `records`。
- `src/table`、`src/wal`、`src/version` 等模块也公开了一些底层 building block。它们对测试和内部工具有用，但日常使用建议优先通过 `DB` API 操作。
