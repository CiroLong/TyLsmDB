# TYLSMDB 实现路线图

本目录把 [../lsm_kv_design.md](../lsm_kv_design.md) 中的整体设计拆成可演进、可验收的版本计划。代码标识符、文件路径、crate 名、测试函数名和必要的存储系统术语保持原样。

## 版本序列

| 版本 | 计划 | 可工作的能力 |
| --- | --- | --- |
| V0 | [工程骨架](v0_project_skeleton.md) | 库 crate、公开 API 形状、选项、错误类型、基础测试框架 |
| V1 | [内存引擎](v1_in_memory_engine.md) | 基于 `BTreeMap` memtable 的 `put/get/delete/scan`，带序列号 |
| V2 | [WAL 与恢复](v2_wal_recovery.md) | 持久化 `WriteBatch` 日志、同步写、重启恢复 |
| V3 | [SSTable 与刷盘](v3_sstable_flush.md) | 数据块格式、SSTable builder/reader、immutable memtable 刷盘 |
| V4 | [VersionSet 与 MANIFEST](v4_versions_manifest.md) | VersionSet、MANIFEST、CURRENT、文件号、跨 SST/WAL 的重启恢复 |
| V5 | [读路径与缓存](v5_read_path_cache.md) | 完整多层读路径、归并迭代器、布隆过滤器、block/table cache |
| V6 | [分层压缩合并与 GC](v6_leveled_compaction_gc.md) | 分层压缩合并、tombstone/version GC、废弃文件清理、写入限流 |
| V7 | [MVCC 与事务](v7_mvcc_transactions.md) | Snapshot、水位线、乐观事务、SSI 校验 |
| V8 | [优化与加固](v8_optimization_hardening.md) | 跳表/arena、压缩、group commit、subcompaction、基准测试、故障测试 |

## 规划规则

- 每个版本完成后，项目都必须能编译，并能通过 `cargo test`。
- 每个版本应独立合入后再进入下一版。
- 本计划面向单进程嵌入式 Rust 库，不面向网络数据库服务。
- 文件路径都相对于仓库根目录。
- 每个版本的“退出条件”就是进入下一版前的交接门槛。

## 依赖关系

```text
V0 -> V1 -> V2 -> V3 -> V4 -> V5 -> V6 -> V7 -> V8
```

这个顺序刻意保持线性。单个版本内部可以并行推进一部分任务，但存储格式与恢复语义必须先稳定，再让上层能力依赖它们。
