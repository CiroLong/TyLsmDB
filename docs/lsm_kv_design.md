# TYLSMDB LSM-Tree KV 存储引擎设计

> 目标：参考 LevelDB、RocksDB 与 skyzh/mini-lsm 的教学架构，为 TYLSMDB 设计一个用 Rust 实现的完整单机嵌入式 LSM-tree KV 存储引擎。本文档描述功能边界、模块划分、核心数据结构、文件格式、读写路径、持久化恢复、压缩合并、MVCC、并发、测试与演进路线。

来源：迁移自 Codex 线程 `019ed06c-b21f-7fe3-b03f-c08a44dc0036` 的设计文档。

实现计划：[implementation/README.md](implementation/README.md)。

说明：正文已中文化；代码标识符、文件名、配置项、API 名称和必要的存储系统术语保持原样，以便和实现计划及后续代码对应。

## 1. 设计目标

### 1.1 产品定位

本项目是一个嵌入式 KV 数据库库，类似 LevelDB/RocksDB 的单机存储引擎，而不是网络数据库服务。

优先目标：

- 支持二进制 key/value：`&[u8]` / `Vec<u8>`。
- 支持 `put`、`delete`、`get`、`write_batch`、`scan`。
- 支持持久化、崩溃恢复、WAL、MANIFEST、SSTable。
- 支持 LSM 分层压缩合并，并预留分层聚合与 universal compaction。
- 支持 snapshot read、事务 API、乐观并发控制、可串行化快照隔离。
- 支持 block cache、Bloom filter、前缀压缩、checksum、压缩。
- 提供清晰的 Rust 模块边界和可测试架构。

非目标：

- 第一阶段不实现分布式复制、Raft、多副本一致性。
- 第一阶段不实现 SQL、二级索引、查询优化器。
- 第一阶段不追求 RocksDB 全量特性，如列族、merge operator、备份引擎、远程 compaction；这些作为扩展能力预留。

### 1.2 参考来源

- mini-lsm 将实现路线拆为三部分：存储格式与引擎骨架、compaction 与持久化、MVCC。我们的路线沿用这个学习曲线，但设计为完整工程能力。
- LevelDB 的实现文档明确了 WAL、memtable、SSTable、level、MANIFEST、CURRENT、compaction、恢复、废弃文件 GC 等基础结构。
- RocksDB 的 leveled compaction 文档补充了 L0 特殊性、非 L0 层有序 run、不重叠文件范围、level 目标大小、compaction 分数、后台 compaction 并行化等工业经验。

## 2. 总体架构

```text
TYLSMDB
├── 公开 API
│   ├── DB
│   ├── ReadOptions / WriteOptions / Options
│   ├── Iterator / Snapshot / Transaction
│   └── 管理 API
├── 写路径
│   ├── WriteBatch
│   ├── WriteGroup
│   ├── WAL
│   ├── Mutable MemTable
│   └── Immutable MemTables
├── 读路径
│   ├── MemTable reader
│   ├── SSTable reader
│   ├── MergeIterator / TwoMergeIterator / ConcatIterator
│   ├── Bloom filter
│   └── Block cache
├── 存储格式
│   ├── Data block
│   ├── Index block
│   ├── Filter block
│   ├── Meta block
│   ├── Footer
│   └── Checksums / Compression
├── 版本管理
│   ├── Version
│   ├── VersionSet
│   ├── MANIFEST
│   ├── CURRENT
│   └── 文件号分配器
├── 后台工作线程
│   ├── flush worker
│   ├── compaction picker
│   ├── compaction executor
│   ├── 废弃文件清理器
│   └── 统计与错误传播
└── MVCC
    ├── sequence number
    ├── snapshot
    ├── transaction record
    ├── watermark
    └── 垃圾回收
```

## 3. 对外 API

### 3.1 基础 DB 接口

```rust
pub struct DB {
    inner: Arc<DBInner>,
}

impl DB {
    pub fn open(path: impl AsRef<Path>, options: Options) -> Result<Self>;
    pub fn close(&self) -> Result<()>;

    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()>;
    pub fn delete(&self, key: &[u8]) -> Result<()>;
    pub fn write(&self, batch: WriteBatch, opts: WriteOptions) -> Result<()>;

    pub fn get(&self, key: &[u8]) -> Result<Option<Bytes>>;
    pub fn get_opt(&self, key: &[u8], opts: ReadOptions) -> Result<Option<Bytes>>;
    pub fn scan(&self, lower: Bound<&[u8]>, upper: Bound<&[u8]>) -> Result<DBIterator>;

    pub fn snapshot(&self) -> Snapshot;
    pub fn transaction(&self, opts: TransactionOptions) -> Result<Transaction>;

    pub fn flush(&self) -> Result<()>;
    pub fn compact_range(&self, lower: Bound<&[u8]>, upper: Bound<&[u8]>) -> Result<()>;
    pub fn sync_wal(&self) -> Result<()>;
}
```

### 3.2 选项

```rust
pub struct Options {
    pub create_if_missing: bool,
    pub error_if_exists: bool,

    pub memtable_size: usize,
    pub max_immutable_memtables: usize,
    pub block_size: usize,
    pub target_file_size_base: usize,
    pub max_levels: usize,
    pub level0_file_num_compaction_trigger: usize,
    pub level0_slowdown_writes_trigger: usize,
    pub level0_stop_writes_trigger: usize,
    pub max_bytes_for_level_base: usize,
    pub max_bytes_for_level_multiplier: f64,

    pub wal_enabled: bool,
    pub wal_sync: WalSyncMode,
    pub compression: CompressionType,
    pub checksum: ChecksumType,
    pub bloom_false_positive_rate: f64,
    pub block_cache_capacity: usize,

    pub max_background_flushes: usize,
    pub max_background_compactions: usize,
    pub max_subcompactions: usize,
}

pub struct WriteOptions {
    pub sync: bool,
    pub disable_wal: bool,
}

pub struct ReadOptions {
    pub snapshot: Option<Snapshot>,
    pub verify_checksums: bool,
    pub fill_cache: bool,
    pub total_order_seek: bool,
}
```

### 3.3 写入批次

```rust
pub enum BatchRecord {
    Put { key: Bytes, value: Bytes },
    Delete { key: Bytes },
}

pub struct WriteBatch {
    records: Vec<BatchRecord>,
}
```

Batch 是原子写入单位：同一个 batch 内所有 record 分配连续 sequence number，WAL append 成功后才进入 memtable。恢复时 batch 也必须按原子单位 replay。

## 4. Key 编码与 MVCC

### 4.1 UserKey 与 InternalKey

外部用户只看到 `UserKey = bytes`。内部所有 memtable 与 SSTable 都存储 `InternalKey`：

```text
InternalKey = user_key | timestamp_or_sequence | value_type
```

排序规则：

```text
user_key 升序
sequence_number 降序
value_type 用于同 sequence 下稳定排序
```

这样同一个 user key 的最新版本会排在最前，读取时找到第一条 `seq <= read_seq` 的记录即可。

### 4.2 ValueType

```rust
pub enum ValueType {
    Put = 1,
    Delete = 2,
    TransactionBegin = 3,
    TransactionCommit = 4,
    TransactionRollback = 5,
}
```

基础 KV 只需要 `Put/Delete`。事务与 SSI 阶段可扩展专用 record，或把事务元信息存在独立 system key namespace。

### 4.3 序列号

`VersionSet` 维护全局 `last_sequence`。写入流程中：

1. 获取 write mutex。
2. 为 batch 分配 `[start_seq, end_seq]`。
3. 将 batch 编码进 WAL。
4. 插入 memtable。
5. 发布 `last_sequence = end_seq`。

Snapshot 保存 `read_seq`，读操作只能看到 `seq <= read_seq` 的版本。

## 5. 内存结构

### 5.1 MemTable

MVP 使用 `crossbeam_skiplist::SkipMap<InternalKey, Bytes>` 或 `BTreeMap<InternalKey, Bytes>`。建议：

- 第一版用 `BTreeMap` 降低实现复杂度。
- 第二版改为 skiplist，支持无锁读、并发写较友好。
- 第三版引入 arena allocator，减少小对象分配成本。

```rust
pub struct MemTable {
    id: FileNumber,
    map: MemIndex,
    wal: Option<WalWriter>,
    approximate_size: AtomicUsize,
    min_seq: SequenceNumber,
    max_seq: SequenceNumber,
}
```

MemTable 状态：

- mutable memtable：当前写入。
- immutable memtables：冻结后等待 flush，按新到旧排列。

### 5.2 冻结与刷盘

当 mutable memtable 超过 `memtable_size`：

1. 创建新 WAL 与新 memtable。
2. 在 `state_lock` 下切换 mutable memtable。
3. 旧 memtable 进入 immutable 队列。
4. 写 MANIFEST `NewMemtable`。
5. 唤醒 flush worker。

Flush worker 将最老 immutable memtable 写成 L0 SSTable。若使用 tiered compaction，也可以写成新的 tier。

## 6. WAL 设计

### 6.1 WAL 文件

文件命名：

```text
000001.wal
000002.wal
```

WAL record：

```text
| crc32c: u32 | length: u32 | type: u8 | payload: [u8] |
```

`type`：

- `FULL`
- `FIRST`
- `MIDDLE`
- `LAST`

大 batch 可切分到多个 fragment，参考 LevelDB log format 思路。

### 6.2 WAL 载荷

```text
| batch_count: u32 |
| start_sequence: u64 |
| repeated records |

record:
| value_type: u8 |
| key_len: varint |
| key |
| value_len: varint |   // Delete 无 value
| value |
```

### 6.3 WAL 同步策略

- `sync = true`：写 WAL 后 `fsync`，提供崩溃持久化。
- `sync = false`：依赖 OS page cache，提高吞吐，但掉电可能丢失最近写入。
- 支持 group commit：多个 writer 聚合成一个 WAL append + fsync。

### 6.4 恢复

启动流程：

1. 获取 DB 目录锁。
2. 读取 `CURRENT` 得到 MANIFEST 文件名。
3. replay MANIFEST，恢复 VersionSet。
4. 找出仍未 flush 的 WAL。
5. replay WAL 到 memtable。
6. 必要时将旧 WAL 恢复出的 memtable flush 成 L0。
7. 创建新的 WAL 与 mutable memtable。
8. 清理 obsolete files。

## 7. SSTable 格式

### 7.1 文件布局

```text
SSTable
├── data block 0
├── data block 1
├── ...
├── filter block
├── index block
├── properties block
├── metaindex block
└── footer
```

Footer 固定长度，包含：

```text
| metaindex_handle | index_handle | checksum_type | format_version | magic |
```

BlockHandle：

```text
| offset: varint64 | size: varint64 |
```

### 7.2 数据块

Block 内 key 按 InternalKey 有序。使用 restart point 做前缀压缩：

```text
entry:
| shared_key_len: varint |
| unshared_key_len: varint |
| value_len: varint |
| unshared_key |
| value |

tail:
| restart_offsets: [u32] |
| restart_count: u32 |
```

每隔 `block_restart_interval` 条记录完整保存一次 key。这样：

- 顺序扫描快。
- 块内二分可先定位 restart point，再线性搜索。
- 格式接近 LevelDB/RocksDB block-based table 的核心思想。

### 7.3 索引块

Index block 保存每个 data block 的分界 key 与 block handle：

```text
index_entry:
| separator_internal_key |
| block_handle |
```

`separator_internal_key` 使用当前 block 最大 key 与下一 block 最小 key 的最短分隔 key，减少 index size。

### 7.4 过滤块

每个 SSTable 或每个 data block 生成 Bloom filter：

```text
filter_key = user_key
hash = farmhash / xxhash / ahash stable variant
```

建议第一版做 table-level Bloom，第二版改为 partitioned filter 或 block-based filter。

### 7.5 属性块

保存统计信息：

```text
num_entries
num_deletions
raw_key_size
raw_value_size
data_size
index_size
filter_size
smallest_key
largest_key
smallest_seq
largest_seq
creation_time
compression
checksum
```

### 7.6 校验和与压缩

Block trailer：

```text
| compression_type: u8 | checksum: u32 |
```

支持：

- `NoCompression`
- `Snappy`
- `Lz4`
- `Zstd`

MVP 可先实现 `NoCompression + crc32c`，接口预留压缩。

## 8. VersionSet、MANIFEST 与文件管理

### 8.1 DB 目录结构

```text
LOCK
CURRENT
MANIFEST-000001
000001.wal
000002.sst
LOG
OPTIONS-000001
```

### 8.2 Version

```rust
pub struct Version {
    pub l0_files: Vec<FileMeta>,       // newest -> oldest, ranges may overlap
    pub levels: Vec<Vec<FileMeta>>,    // L1+ sorted by smallest_key, ranges do not overlap
}

pub struct FileMeta {
    pub number: FileNumber,
    pub file_size: u64,
    pub smallest: InternalKey,
    pub largest: InternalKey,
    pub smallest_seq: SequenceNumber,
    pub largest_seq: SequenceNumber,
    pub being_compacted: bool,
}
```

DBState 使用 copy-on-write：

```rust
pub struct DBState {
    pub mutable: Arc<MemTable>,
    pub immutables: Vec<Arc<MemTable>>,
    pub version: Arc<Version>,
}
```

读路径只需 clone 一个 `Arc<DBState>`，随后无须持有全局锁。

### 8.3 MANIFEST 记录

```rust
pub enum VersionEdit {
    ComparatorName(String),
    LogNumber(FileNumber),
    PrevLogNumber(FileNumber),
    NextFileNumber(FileNumber),
    LastSequence(SequenceNumber),
    AddFile { level: usize, meta: FileMeta },
    DeleteFile { level: usize, number: FileNumber },
    NewMemtable { number: FileNumber },
    Flush { memtable_number: FileNumber, file_number: FileNumber },
    Compaction { inputs: Vec<FileNumber>, outputs: Vec<FileMeta> },
}
```

MANIFEST 是 append-only log。每次状态变更先写入 MANIFEST 并 sync，再发布到内存状态，或采用严格定义的 recoverable 顺序，保证崩溃后不会引用不存在文件。

### 8.4 CURRENT

`CURRENT` 是文本文件，内容为当前 MANIFEST 文件名：

```text
MANIFEST-000001\n
```

更新 CURRENT 时使用临时文件 + rename：

```text
CURRENT.tmp -> fsync -> rename CURRENT -> fsync dir
```

## 9. 读路径

### 9.1 点查

流程：

```text
read_seq = options.snapshot.read_seq or current_last_sequence
snapshot = state.clone()

1. mutable memtable
2. immutable memtable，newest -> oldest
3. L0 文件，newest -> oldest，检查所有重叠范围
4. L1+ 每层最多一个重叠文件
5. 解码最新可见版本
```

L0 因为文件范围可能重叠，必须查多个文件。L1+ 由于同层 key range 不重叠，每层最多定位一个文件。

Bloom filter 只对 SSTable 生效：

```text
if key not in file range -> 跳过
if bloom says no -> 跳过
seek index -> seek data block -> 比较 user key 和 seq
```

### 9.2 范围扫描

Scan 需要把多个有序源归并：

```text
memtable iterator
immutable memtable iterators
L0 SST iterators
L1+ concat iterators
```

核心 iterator：

- `StorageIterator`：统一接口。
- `MergeIterator`：多路归并。
- `TwoMergeIterator`：两个 iterator 按 key merge，左侧优先。
- `SstConcatIterator`：同层非重叠 SST 顺序串接。
- `DBIterator`：过滤 tombstone、旧版本、超出 snapshot 的版本。

同一 user key 多版本处理：

```text
for entries with same user_key:
    choose first entry with seq <= read_seq
    Put => return
    Delete => skip key
    skip older versions
```

## 10. 写路径

### 10.1 单条写

`put/delete` 都转换为 `WriteBatch`：

```text
DB::put
-> WriteBatch::put
-> DB::write
```

### 10.2 批量写入流程

```text
1. 获取 write_mutex
2. 如果 L0 文件或 immutable memtable 过多，则可能触发写入等待
3. 分配 sequence number
4. 追加 WAL
5. 如请求同步，则同步 WAL
6. 将记录插入 mutable memtable
7. 发布 last_sequence
8. 必要时冻结 memtable
9. 释放 write_mutex
10. 唤醒 flush/compaction worker
```

### 10.3 写入限流

为避免 compaction 跟不上写入，需要限速：

- immutable memtable 数超过阈值：等待 flush。
- L0 文件数超过 `slowdown_writes_trigger`：写入 sleep/backoff。
- L0 文件数超过 `stop_writes_trigger`：阻塞直到 compaction 推进。

### 10.4 原子性

Batch 原子性通过 WAL + memtable 顺序保证：

- WAL 中 batch 是一个逻辑 record。
- replay 时 batch 内记录全部恢复。
- memtable 插入失败时 DB 进入 background error，不继续接收写入。

## 11. 刷盘

Flush 将 immutable memtable 转成 L0 SSTable：

```text
1. 选择最老的 immutable memtable
2. 创建 SSTableBuilder
3. 遍历 memtable entry
4. 写入 data/index/filter/properties/footer
5. fsync SSTable
6. 写入 MANIFEST AddFile / Flush
7. 发布新 Version
8. 移除已 flush 的 immutable memtable
9. 删除废弃 WAL
10. fsync 目录
```

Flush 输出文件范围：

- `smallest/largest` 是 InternalKey。
- 额外保存 `smallest_user_key/largest_user_key` 便于 overlap 计算。

## 12. 压缩合并

### 12.1 压缩合并类型

支持三类策略：

```rust
pub enum CompactionStyle {
    NoCompaction,
    Leveled,
    Tiered,
    Universal,
}
```

第一条主线实现 Leveled，之后补 Tiered/Universal。

### 12.2 分层压缩合并结构

```text
L0: flushed SSTs, newest -> oldest, ranges may overlap
L1: sorted run, non-overlapping ranges
L2: sorted run, non-overlapping ranges
...
Ln: sorted run, non-overlapping ranges
```

目标大小：

```text
target(L1) = max_bytes_for_level_base
target(Ln+1) = target(Ln) * max_bytes_for_level_multiplier
```

可后续支持 RocksDB 风格 dynamic level bytes。

### 12.3 压缩合并分数

```text
score(L0) = max(
    l0_file_count / level0_file_num_compaction_trigger,
    l0_total_size / max_bytes_for_level_base
)

score(Ln) = level_size / target(Ln), n >= 1
```

选择 score 最大且 `score >= 1` 的 level compact。

### 12.4 选择压缩合并任务

L0 -> L1：

- 选择一组 L0 文件。
- 因 L0 范围可能互相重叠，需要扩展选择所有重叠 L0 文件。
- 找出 L1 中与输入范围重叠的文件。

Ln -> L(n+1)，n >= 1：

- 选择一个 Ln 文件。
- 找出 L(n+1) 中所有重叠文件。
- 可使用每层 compact pointer，在 key 空间中轮转，避免一直 compact 热区。

### 12.5 压缩合并执行

```text
1. build input iterators
2. merge all records by InternalKey
3. apply snapshot/version/tombstone GC rules
4. apply compaction filters
5. split output by target_file_size_base
6. fsync output SSTs
7. write MANIFEST DeleteFile + AddFile
8. publish new Version
9. obsolete file GC
```

### 12.6 丢弃规则

对同一 user key 的多版本：

- 保留所有 `seq > oldest_snapshot_seq` 的版本。
- 对 `seq <= oldest_snapshot_seq`，只保留最新可见版本。
- 如果最新可见版本是 Delete，且更深层不存在同 key range 的旧数据，可以删除 tombstone。
- 如果更深层可能仍有旧值，必须保留 tombstone 直到 compact 到 bottommost level 或确认无 overlap。

### 12.7 平凡移动

当 Ln 文件与 L(n+1) 没有 overlap，可直接把文件元数据移动到下一层，无须重写 SSTable。

### 12.8 子压缩合并

对 L0 -> L1 的大 compaction，可按 user key range 切分成多个 subcompaction：

```text
[a, f), [f, k), [k, z)
```

每个子任务独立 merge 并输出 SST。最终一次性提交 VersionEdit。

## 13. MVCC、Snapshot 与事务

### 13.1 Snapshot 读

`Snapshot` 保存：

```rust
pub struct Snapshot {
    read_seq: SequenceNumber,
    inner: Arc<SnapshotInner>,
}
```

创建 snapshot 时加入 snapshot list，释放时移除。`oldest_snapshot_seq` 用于 compaction GC。

### 13.2 水位线

维护活跃读事务最小 sequence：

```text
oldest_snapshot_seq = min(active_snapshots.read_seq)
```

没有活跃 snapshot 时，等于 `last_sequence`。

### 13.3 事务 API

```rust
pub struct Transaction {
    db: Arc<DBInner>,
    read_seq: SequenceNumber,
    writes: WriteBatch,
    read_set: Vec<KeyRange>,
    write_set: Vec<Bytes>,
}

impl Transaction {
    pub fn get(&mut self, key: &[u8]) -> Result<Option<Bytes>>;
    pub fn put(&mut self, key: &[u8], value: &[u8]);
    pub fn delete(&mut self, key: &[u8]);
    pub fn scan(&mut self, lower: Bound<&[u8]>, upper: Bound<&[u8]>) -> Result<TxnIterator>;
    pub fn commit(self) -> Result<()>;
    pub fn rollback(self);
}
```

### 13.4 乐观并发控制

提交时：

1. 获取 commit mutex。
2. 检查 read_set 中是否存在 `seq > read_seq` 的已提交写入。
3. 检查 write-write conflict。
4. 若无冲突，分配 commit sequence。
5. 写 WAL + memtable。

### 13.5 可串行化快照隔离

可用两阶段实现：

- 第一阶段：快照隔离，只检测写写冲突。
- 第二阶段：可串行化快照隔离，额外检测读写冲突和范围幻读。

Range scan 记录 predicate/range：

```text
ReadSet:
    Point(key)
    Range(lower, upper)
```

提交时检查新写入是否落入活跃事务读过的 range。

## 14. Block Cache 与 Table Cache

### 14.1 Block Cache

```rust
type BlockCacheKey = (FileNumber, BlockOffset);
type BlockCache = moka::sync::Cache<BlockCacheKey, Arc<Block>>;
```

支持：

- data block cache
- index/filter cache 可选
- `ReadOptions.fill_cache = false` 时不污染 cache

### 14.2 Table Cache

缓存打开的 SSTable reader：

```rust
type TableCache = LruCache<FileNumber, Arc<SsTable>>;
```

避免每次读都 open 文件。GC 文件时需要确保旧 reader 引用释放后再删除底层文件，或者采用 pending delete。

## 15. 并发模型

### 15.1 锁分层

```text
write_mutex:
    串行化 sequence 分配、WAL append、memtable insert

state_lock:
    保护 DBState 切换、Version 发布、memtable freeze/flush/compaction commit

manifest_lock:
    串行化 MANIFEST append

snapshot_mutex:
    管理活跃 snapshot watermark
```

读路径原则：

- 获取 `Arc<DBState>` 后立即释放锁。
- MemTable、SSTable、Version 都是 immutable 或读并发安全。
- Iterator 持有 snapshot/version 引用，保证文件不会被过早删除。

### 15.2 后台工作线程

线程：

- flush thread pool
- compaction thread pool
- cleanup thread

所有后台错误写入 `background_error`：

```rust
Atomic<Option<Error>>
```

一旦发生不可恢复后台错误，新的写入返回错误，避免继续扩大损坏。

## 16. 文件删除与 GC

### 16.1 废弃文件

可删除文件：

- 不属于当前 Version 或任何活跃 iterator/snapshot 引用的 SST。
- 已 flush 且不再需要 recovery 的 WAL。
- 旧 MANIFEST、旧 OPTIONS。
- 临时文件。

### 16.2 删除时机

调用点：

- DB open recovery 完成后。
- flush 完成后。
- compaction 完成后。
- manual cleanup。

为避免读 iterator 正在使用文件：

- Version 引用计数未归零前不删除，或
- rename 到 pending-delete 后延迟删除。

## 17. 错误处理与一致性

### 17.1 错误类型

```rust
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("corruption: {0}")]
    Corruption(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("background error: {0}")]
    Background(String),
    #[error("transaction conflict")]
    TransactionConflict,
    #[error("database closed")]
    Closed,
}
```

### 17.2 崩溃一致性规则

关键顺序：

- 写入：WAL durable 后才能向用户确认同步写入成功。
- Flush：SST durable 后才能把 AddFile 写入 MANIFEST。
- Compaction：输出 SST durable 后才能通过 MANIFEST 删除旧文件并添加新文件。
- CURRENT 更新必须使用原子 rename。
- MANIFEST replay 必须允许重复、缺失中间临时文件、崩溃半写 record。

## 18. 模块划分

```text
src/
├── lib.rs
├── db.rs
├── options.rs
├── error.rs
├── bytes.rs
├── key/
│   ├── mod.rs
│   ├── internal_key.rs
│   └── comparator.rs
├── memtable/
│   ├── mod.rs
│   ├── btree.rs
│   ├── skiplist.rs
│   └── arena.rs
├── wal/
│   ├── mod.rs
│   ├── writer.rs
│   ├── reader.rs
│   └── format.rs
├── table/
│   ├── mod.rs
│   ├── builder.rs
│   ├── reader.rs
│   ├── block.rs
│   ├── block_builder.rs
│   ├── block_iterator.rs
│   ├── filter.rs
│   ├── properties.rs
│   └── format.rs
├── version/
│   ├── mod.rs
│   ├── version.rs
│   ├── version_set.rs
│   ├── manifest.rs
│   └── edit.rs
├── compact/
│   ├── mod.rs
│   ├── picker.rs
│   ├── task.rs
│   ├── leveled.rs
│   ├── tiered.rs
│   └── executor.rs
├── iterator/
│   ├── mod.rs
│   ├── storage_iterator.rs
│   ├── merge_iterator.rs
│   ├── two_merge_iterator.rs
│   ├── concat_iterator.rs
│   └── db_iterator.rs
├── mvcc/
│   ├── mod.rs
│   ├── snapshot.rs
│   ├── transaction.rs
│   ├── watermark.rs
│   └── conflict.rs
├── cache/
│   ├── mod.rs
│   ├── block_cache.rs
│   └── table_cache.rs
├── env/
│   ├── mod.rs
│   ├── fs.rs
│   └── file.rs
└── util/
    ├── coding.rs
    ├── crc.rs
    ├── bloom.rs
    └── rate_limiter.rs
```

## 19. 测试策略

### 19.1 单元测试

- InternalKey 编码与排序。
- varint 编解码。
- WAL record fragment 与 checksum。
- BlockBuilder prefix compression。
- SSTableBuilder/Reader roundtrip。
- Bloom filter false negative 必须为 0。
- MergeIterator 多源归并顺序。

### 19.2 集成测试

- `put/get/delete`。
- batch 原子性。
- scan range 边界。
- memtable freeze + flush。
- WAL recovery。
- MANIFEST recovery。
- L0/L1/Ln compaction correctness。
- tombstone GC。
- snapshot 隔离。
- transaction conflict。
- iterator 在 compaction 并发时稳定读取。

### 19.3 崩溃测试

引入 fault injection：

```rust
pub trait Env {
    fn write(&self, ...) -> Result<()>;
    fn sync(&self, ...) -> Result<()>;
    fn rename(&self, ...) -> Result<()>;
}
```

在以下点注入崩溃：

- WAL append half record。
- WAL sync 前后。
- SST 写一半。
- SST sync 后 MANIFEST 前。
- MANIFEST 写一半。
- MANIFEST sync 后 Version 发布前。
- CURRENT rename 前后。
- obsolete file 删除中。

每次重启验证：

- 无已确认 sync 写丢失。
- 可能丢失未 sync 写。
- 不返回已删除的旧值。
- scan 有序且无重复 user key。

### 19.4 模糊测试与模型测试

用 `BTreeMap<Vec<u8>, Vec<u8>>` 作为 oracle：

- 随机 put/delete/get/scan。
- 随机 flush/compaction/reopen。
- 随机 snapshot/transaction。
- 与 oracle 对比可见结果。

## 20. 基准测试

### 20.1 微基准

- 顺序写入
- 随机写入
- 点查命中/未命中
- 范围扫描
- WAL 同步写延迟
- flush 吞吐
- compaction 吞吐

### 20.2 指标

```text
write amplification = bytes_written_to_sst / user_bytes_written
read amplification = files_or_blocks_touched_per_get
space amplification = live_sst_bytes / live_user_bytes
p50/p95/p99 延迟
block cache 命中率
bloom filter 有效性
待处理 compaction 字节数
```

## 21. 实现路线

### 阶段 0：工程骨架

- Cargo workspace。
- Error/Options/DB API。
- Bytes、InternalKey、coding utilities。
- 基础测试框架。

### 阶段 1：内存 KV

- MemTable。
- sequence number。
- tombstone。
- get/scan。

### 阶段 2：WAL 与恢复

- WAL writer/reader。
- WriteBatch。
- 同步写。
- replay 恢复。

### 阶段 3：SSTable

- block 格式。
- SSTable builder/reader。
- table iterator。
- flush immutable memtable。

### 阶段 4：VersionSet 与 MANIFEST

- VersionEdit。
- MANIFEST append/replay。
- CURRENT。
- 文件号分配器。
- reopen 后恢复 SST/WAL。

### 阶段 5：读路径完整化

- L0 + L1+ search。
- MergeIterator / ConcatIterator。
- DBIterator 过滤多版本和 tombstone。
- Bloom filter。
- block cache。

### 阶段 6：分层压缩合并

- compaction 分数。
- picker。
- executor。
- tombstone/version GC。
- 废弃文件清理。
- 写入限流。

### 阶段 7：MVCC

- snapshot read。
- watermark。
- transaction API。
- 乐观冲突检测。
- 可串行化快照隔离。

### 阶段 8：优化

- skiplist + arena。
- 前缀压缩。
- 压缩。
- table cache。
- group commit。
- subcompaction。
- compaction filter。
- rate limiter。
- metrics。

## 22. 推荐默认参数

开发测试：

```text
memtable_size = 4 MiB
block_size = 4 KiB
target_file_size_base = 2 MiB
max_levels = 7
level0_file_num_compaction_trigger = 4
level0_slowdown_writes_trigger = 12
level0_stop_writes_trigger = 20
max_bytes_for_level_base = 10 MiB
max_bytes_for_level_multiplier = 10
block_cache_capacity = 256 MiB
bloom_false_positive_rate = 0.01
```

教学/测试可调小：

```text
memtable_size = 64 KiB
target_file_size_base = 64 KiB
max_bytes_for_level_base = 256 KiB
```

## 23. 关键设计决策

1. 第一版坚持同步 API，不引入 async runtime。
2. 使用 copy-on-write `Arc<DBState>`，让读路径无长时间全局锁。
3. InternalKey 从第一天支持 sequence number，为 snapshot 和 compaction GC 铺路。
4. WAL、MANIFEST、SSTable 都使用 checksum，默认检测 corruption。
5. L0 特殊处理，L1+ 保证同层不重叠，这是读放大可控的基础。
6. batch 是原子写入单位，事务提交复用 batch。
7. compaction 与 flush 的提交都通过 VersionEdit 原子切换。
8. tombstone GC 必须受 snapshot watermark 与下层 overlap 共同约束。
9. 所有文件生命周期由 VersionSet 管理，不能让后台线程直接随意删除文件。
10. 先做正确性，再做 RocksDB 风格性能优化。

## 24. 参考链接

- skyzh/mini-lsm: https://github.com/skyzh/mini-lsm
- mini-lsm 书籍：https://skyzh.github.io/mini-lsm/
- mini-lsm 概览：https://skyzh.github.io/mini-lsm/00-overview.html
- LevelDB 实现说明：https://github.com/google/leveldb/blob/main/doc/impl.md
- RocksDB 分层压缩合并：https://github.com/facebook/rocksdb/wiki/Leveled-Compaction
