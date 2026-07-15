# TYLSMDB 目标路线

本文只保留当前项目仍有价值的目标路线：TYLSMDB 的定位、已经完成的能力，以及后续可以继续推进的方向。历史逐版本实施稿已经合并为下面的能力主线。

## 定位

TYLSMDB 是单机嵌入式 LSM tree key-value 存储引擎，目标是作为 Rust 应用内的本地 storage library 使用。它对外暴露 bytes key/value API，不提供网络协议、SQL、认证授权或分布式复制。

设计参考 LevelDB、RocksDB 和 mini-lsm 的核心结构，但实现范围保持克制：优先完成一条可恢复、可压缩合并、可测试的 LSM 主路径。

## 已完成主线

1. **公开 API 与内存引擎**：`DB`、`Options`、`ReadOptions`、`WriteOptions`、`WriteBatch`，以及基于 sequence number 的 put/get/delete/scan。
2. **WAL 与恢复**：WAL record checksum、partial record EOF 语义、corrupt complete record 报错、显式 sync、per-write sync 和 reopen replay。
3. **SSTable 与 flush**：data block prefix compression、Bloom filter、properties、index/footer、block checksum、zstd block compression，以及 memtable flush 到 L0。
4. **VersionSet 与 MANIFEST**：`CURRENT`、`MANIFEST-*`、file number、active WAL number、last sequence 和跨 SST/WAL recovery。
5. **读路径与缓存**：memtable、immutable memtable、L0、lower levels 的点查与 scan，block cache、table cache 和 streaming scan iterator。
6. **Leveled compaction**：compaction picker/executor、manual compaction、tombstone/version GC、obsolete file cleanup、L0 写入压力处理和 subcompaction 输出拆分。
7. **MVCC 与事务**：snapshot read、水位线、乐观事务、read-your-own-writes、write/write、read/write 和 range phantom conflict 检测。
8. **优化与加固**：SkipList memtable、arena、metrics、group commit、write rate limiter、模型测试、fuzz harness、crash recovery 故障注入测试和 benchmarks。
9. **API 语义加固**：`ReadOptions.fill_cache` 语义、`Env` 读写边界、close 后错误处理、测试文件按模块重构。

## 后续方向

短期优先级：

- 让 `ReadOptions.verify_checksums` 和 `ReadOptions.total_order_seek` 具备实际行为，或从公开 API 中明确移除。
- 让 `Options.bloom_false_positive_rate`、`max_immutable_memtables` 和后台 worker 相关选项真正驱动主路径。
- 补充更稳定的 pressure benchmark binary，覆盖长稳态写入、读写混合、恢复耗时和 tail latency。
- 改善 metrics：flush/compaction 次数与耗时、stall 时间、目录总大小、读放大和空间放大。

中期方向：

- 后台 flush/compaction worker 和后台错误传播。
- 更完整的 compaction 策略，例如 tiered/universal compaction 或 dynamic level bytes。
- 更细粒度 Bloom/filter layout，例如 block-based 或 partitioned filter。
- 更明确的 `Env` 扩展点，便于后续接入非本地文件系统。

明确非目标：

- 分布式复制、Raft、多副本一致性。
- SQL、二级索引、查询优化器。
- 认证授权、加密、网络服务层。
- RocksDB 全量兼容能力，例如 column family、merge operator、backup engine、remote compaction。

## 维护原则

- 存储格式变更必须有 recovery 和 corruption tests。
- 读可见性或 sequence 语义变更必须覆盖 MVCC、transaction 和 model tests。
- 文件系统行为必须走 `Env`，让 crash-recovery/fault-injection tests 能观测和注入失败。
- 新增公开 API 需要同步更新 [API 使用说明](usage.md)。
