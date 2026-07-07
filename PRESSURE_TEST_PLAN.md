# TYLSMDB 压力测试方案

本文给出 TYLSMDB 的压力测试设计，用于发现吞吐、延迟、写放大、读放大、缓存效率、compaction stall、恢复时间和长稳运行问题。方案基于当前代码中已有的 `benches/write_read.rs`、`DB::metrics_snapshot()`、`DB::level_file_counts()` 和测试能力，同时参考 RocksDB/LevelDB `db_bench`、YCSB workload、TiKV/RocksDB 监控指标的常见做法。

## 目标

压测要回答的问题：

- 单线程和多线程下，写入、点查、范围扫、混合读写的吞吐和延迟是多少。
- WAL sync、memtable kind、block size、cache size、zstd compression、subcompaction 等选项对性能有什么影响。
- 长时间写入和覆盖更新下，flush/compaction 是否造成明显写停顿。
- 数据量超过内存后，读放大、写放大、block cache 命中率、Bloom filter 有效性如何变化。
- crash/reopen 后恢复时间是否随 WAL 和 MANIFEST 增长而失控。
- 代码变更后是否出现性能回退。

非目标：

- 不做分布式数据库压测。TYLSMDB 是单机嵌入式 library，不测网络、RPC、Raft、SQL。
- 不把 Criterion microbench 当成完整压测。Criterion 适合短路径回归，不适合长稳态和尾延迟。

## 外部参考方法

RocksDB/LevelDB `db_bench` 常见 workload：

- `fillseq`：顺序 key 写入，观察 bulk load 和 WAL/memtable 基础开销。
- `fillrandom`：随机 key 写入，观察 memtable、flush、compaction 路径。
- `overwrite`：对已有 key 随机覆盖，观察多版本堆积和 compaction GC。
- `readrandom`：随机点查命中，观察 block cache、Bloom filter、level 查找。
- `readmissing`：随机点查未命中，观察 Bloom filter 是否有效减少 IO。
- `readseq` / `seekrandom`：顺序扫描和随机 seek，观察迭代器和 block 解码成本。
- `readwhilewriting`：读写并发，观察读延迟被写入、flush、compaction 干扰的程度。

YCSB 常见 workload：

- A：50% read + 50% update，更新密集型混合负载。
- B：95% read + 5% update，读多写少。
- C：100% read，只读点查。
- D：read latest + insert，模拟读取最近写入数据。
- E：scan + insert，短范围扫描混合插入。
- F：read-modify-write，先读后写，适合事务或条件更新压力。

TiKV/RocksDB 常看指标：

- throughput：ops/s、MiB/s。
- latency：平均、p50、p95、p99、p999、max；写路径尤其关注 tail latency。
- amplification：write amplification、read amplification、space amplification。
- cache/filter：block cache hit/miss、Bloom useful/false-positive。
- compaction：compaction read/write bytes、L0 file count、level file count、stall 次数/时间。
- recovery：reopen/replay WAL 时间、恢复后的数据一致性。

## 当前仓库已有基础

已有 Criterion bench 在 `benches/write_read.rs`：

- `sequential_write`
- `random_write`
- `point_lookup_hit`
- `point_lookup_miss`
- `range_scan`
- `wal_sync_write_latency`
- `flush_and_compaction_throughput`
- `amplification_metrics`

已有指标来自 `MetricsSnapshot`：

- `user_write_bytes`
- `wal_write_bytes`
- `wal_sync_count`
- `sst_write_bytes`
- `compaction_read_bytes`
- `compaction_write_bytes`
- `subcompaction_tasks`
- `max_subcompaction_parallelism`
- `block_cache_hits`
- `block_cache_misses`
- `bloom_useful`
- `bloom_false_positive`

已有状态 API：

- `DB::level_file_counts()`
- `DB::block_cache_stats()`
- `DB::metrics_snapshot()`

已有不足：

- Criterion bench 每轮操作数很小，主要适合 micro regression，不足以形成 steady state。
- 没有单独的压测 binary，无法统一指定数据量、线程数、key/value 大小、运行时长、读写比例和输出格式。
- 现有 metrics 缺少 op latency histogram、flush/compaction 次数、stall 时间、DB 文件总大小、open/recovery 时间。
- `DB::scan` 当前返回 `Vec`，大范围 scan 会把结果一次性收集到内存，不适合作为无限流式扫描压测。

## 推荐压测工具形态

新增一个 binary：`src/bin/pressure.rs`。

命令形态建议：

```text
cargo run --release --bin pressure -- \
  --db target/tylsmdb-pressure \
  --workload ycsb-a \
  --records 1000000 \
  --operations 5000000 \
  --threads 8 \
  --key-size 16 \
  --value-size 1024 \
  --read-ratio 50 \
  --update-ratio 50 \
  --scan-ratio 0 \
  --scan-len 100 \
  --sync never \
  --memtable skiplist \
  --compression zstd \
  --block-cache-mb 512 \
  --report-json target/tylsmdb-pressure/report.json
```

为了保持依赖克制，第一版可以只用标准库手写参数解析、随机数生成和直方图。后续如果愿意引入依赖，再考虑 `clap`、`hdrhistogram`、`rand`。

输出建议同时写 human-readable text 和 JSON lines：

- 每秒输出一次 interval stats。
- 结束时输出 summary stats。
- JSON 中包含 workload 配置、机器信息、DB options、吞吐、延迟分位、错误数、metrics snapshot、level file counts、目录大小。

## Workload 矩阵

### 1. 基线 micro workloads

目的：和当前 Criterion bench 对齐，建立快速回归基线。

| 名称 | 数据准备 | 操作 | 指标 |
| --- | --- | --- | --- |
| `fillseq` | 空 DB | 顺序写入 N 条 | ops/s、MiB/s、p99 write latency、WAL bytes、SST bytes |
| `fillrandom` | 空 DB | 随机 key 写入 N 条 | ops/s、p99 write latency、L0 文件数、flush/compaction bytes |
| `overwrite` | 先 load N 条 | 随机覆盖 M 次 | update ops/s、write amplification、space amplification |
| `readrandom-hit` | 先 load N 条并 flush/compact | 随机命中点查 | read ops/s、p99 read latency、cache hit rate |
| `readrandom-miss` | 先 load N 条 | 随机不存在 key 点查 | miss ops/s、Bloom useful、false positive |
| `scan-short` | 先 load N 条 | 随机起点 scan 10/100 条 | scan ops/s、每条返回成本、p99 scan latency |

### 2. YCSB-style 混合 workloads

目的：模拟常见业务读写组合。

| 名称 | 比例 | 建议参数 | 关注点 |
| --- | --- | --- | --- |
| `ycsb-a` | 50% read, 50% update | uniform + zipfian 两种 key 分布 | 写入对读 tail latency 的影响 |
| `ycsb-b` | 95% read, 5% update | cache 分别设为小/大 | cache hit、Bloom、p99 read |
| `ycsb-c` | 100% read | warm cache 和 cold cache 各跑一次 | 点查极限吞吐 |
| `ycsb-d` | read latest + insert | 读最近 key 窗口 | 新写数据读路径、L0 堆积 |
| `ycsb-e` | 95% scan, 5% insert | scan len 10/100/1000 | scan 分配和 block cache 压力 |
| `ycsb-f` | read-modify-write | 可用 transaction 或 get+put 两版 | 事务冲突、读后写延迟 |

当前没有 zipfian 工具函数时，第一版可先做 uniform 和 hotset：

- uniform：key 在 `[0, records)` 均匀分布。
- hotset：80% 请求落在 20% key 上，用简单取模实现。

### 3. 并发读写 workloads

目的：暴露锁竞争、group commit、WAL sync、compaction 对 tail latency 的影响。

- `readwhilewriting`：1 个 writer 持续随机写，N 个 reader 随机点查。
- `many-writers`：N 个 writer 并发写，比较 `WalSyncMode::Never`、`PerWrite`、`WriteOptions::sync`。
- `snapshot-read-while-compact`：持有 snapshot 执行读，同时写入、flush、compact，观察旧版本保留和读延迟。
- `transaction-conflict`：多个线程更新同一 hot key range，统计 transaction conflict rate。

### 4. 长稳态 workloads

目的：让 LSM 进入 steady state，而不是只测内存或少量 L0。

建议最小长稳态：

- 先 load 1M 条，value 1 KiB。
- 执行 30 分钟 `ycsb-a` 或 `overwrite`。
- 每秒记录 interval stats。
- 每分钟记录目录大小、level file counts、metrics delta。

大数据版本：

- 数据集至少大于 block cache 5 倍。
- 数据集至少大于 memtable 100 倍。
- 使用 `--target-size-gb` 控制数据规模，而不是只用 records。

### 5. 恢复和崩溃 workloads

目的：验证持久性配置和恢复耗时。

- `reopen-after-large-wal`：写入大量数据但不 flush，记录 reopen recovery 时间。
- `reopen-after-many-sst`：多轮 flush/compact 后 reopen，记录 MANIFEST replay 和 table open 时间。
- `kill-during-write`：子进程运行 pressure，父进程在随机时间 kill，再 reopen 校验已确认写入语义。
- `fault-env-regression`：复用 `tests/crash_recovery.rs` 的故障点思想，做更大规模操作。

## 指标定义

### Throughput

- `ops_per_sec = successful_ops / elapsed_secs`
- `write_mib_per_sec = user_write_bytes_delta / elapsed_secs / 2^20`
- 按操作类型分别统计：read、write、delete、scan、txn commit。

### Latency

每种 op 单独维护 latency histogram：

- p50
- p95
- p99
- p999
- max
- avg

第一版可用固定桶直方图：

```text
0-1us, 1-2us, 2-4us, ... 1s-2s, >2s
```

后续可以替换为 HDR histogram。

### Amplification

用当前 metrics 近似计算：

```text
logical_write_bytes = user_write_bytes
physical_write_bytes = wal_write_bytes + sst_write_bytes + compaction_write_bytes
write_amplification = physical_write_bytes / max(logical_write_bytes, 1)

estimated_read_bytes = compaction_read_bytes + block_cache_misses * options.block_size
read_amplification = estimated_read_bytes / max(user_read_bytes, 1)
```

补充建议：

- 在 pressure harness 自己统计 `user_read_bytes`。
- 用 DB 目录文件总大小估算 `space_amplification = live_db_bytes / live_user_bytes`。
- 区分 WAL bytes、flush SST bytes、compaction write bytes，便于定位。

### Cache 和 Bloom

- `block_cache_hit_rate = hits / max(hits + misses, 1)`
- `bloom_useful_per_miss`
- `bloom_false_positive_rate = bloom_false_positive / max(bloom_useful + bloom_false_positive, 1)`，这是实现层面的近似口径，不等同理论 false positive rate。

### Compaction 和 stall

当前可用：

- `compaction_read_bytes`
- `compaction_write_bytes`
- `subcompaction_tasks`
- `max_subcompaction_parallelism`
- `level_file_counts`

建议新增 metrics：

- flush count、flush bytes、flush duration histogram。
- compaction count、compaction duration histogram。
- write slowdown count/time。
- write stop/forced compaction count/time。
- active L0 file count max。

### Recovery

- open elapsed time。
- WAL bytes replayed。
- WAL records replayed。
- MANIFEST edits replayed。
- recovered memtable entries。
- first successful get latency after open。

这些需要后续在 recovery 路径补充 metrics 或在 pressure harness 用目录扫描近似。

## 测试环境控制

每次压测记录：

- git commit 或 `git status --short`。
- build mode：必须用 `cargo run --release` 或 release binary。
- Rust version。
- OS、CPU 型号、核心数、内存。
- 存储设备类型和挂载路径。
- DB path 是否为空目录。
- 是否 cold cache：如需严格 cold cache，应在独立机器或明确 drop OS page cache 后运行；本仓库默认不要求自动 drop cache。
- TYLSMDB options 完整 dump。

压测目录统一使用 `target/tylsmdb-pressure/<run-id>`，每次 run 生成：

```text
config.json
interval.jsonl
summary.json
summary.txt
```

## 推荐执行组合

### 快速本地回归

目标：开发机 1-3 分钟跑完，作为 PR 前快速检查。

```text
cargo bench --bench write_read
cargo run --release --bin pressure -- --workload fillseq --records 100000 --operations 100000
cargo run --release --bin pressure -- --workload readrandom-hit --records 100000 --operations 200000
cargo run --release --bin pressure -- --workload ycsb-a --records 100000 --operations 200000 --threads 4
```

门禁建议：

- 与 main 分支同机同参数对比，吞吐下降超过 15% 或 p99 上升超过 25% 需要解释。
- 任何 workload 出现 panic、corruption、数据校验失败直接阻断。

### 每日长稳压测

目标：发现 compaction、GC、cache、stall 问题。

```text
pressure fillrandom records=5M value=1KiB
pressure overwrite records=5M operations=20M threads=8 duration=30m
pressure ycsb-a records=5M operations=20M threads=8 duration=30m
pressure ycsb-e records=2M scan_len=100 operations=5M threads=4
pressure reopen-after-large-wal records=2M
```

门禁建议：

- 无数据一致性错误。
- p99 写延迟没有持续性阶梯上升。
- L0 file count 不长期贴近 `level0_stop_writes_trigger`。
- space amplification 不持续单调增长。
- reopen time 在同等 WAL/MANIFEST 规模下不退化超过 20%。

### 发布前压测

目标：覆盖配置矩阵和更大数据规模。

矩阵维度：

- memtable：`BTree` vs `SkipList`
- compression：`None` vs `Zstd`
- WAL：`Never` vs `PerWrite` vs per-write `WriteOptions::sync`
- block cache：64 MiB vs 512 MiB vs 数据集 10%
- value size：100 B、1 KiB、8 KiB
- key distribution：uniform vs hotset
- threads：1、4、8、16

## Harness 实现建议

### 模块拆分

建议新增：

```text
src/bin/pressure.rs
```

内部结构：

- `Config`：命令行参数和默认值。
- `Workload`：枚举和比例配置。
- `KeyGenerator`：sequential、uniform、hotset、latest。
- `ValueGenerator`：固定长度 value。
- `Stats`：每线程本地计数和 latency buckets。
- `Reporter`：interval 和 summary 输出。
- `Verifier`：可选抽样校验；小规模可用 `BTreeMap` oracle，大规模用写入日志抽样。

### 随机数

为避免引入依赖，第一版可用简单 xorshift64：

```rust
struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}
```

### Latency buckets

第一版使用 64 个指数桶，线程本地统计，结束时合并，避免每个 op 加全局锁。

### 并发模型

- 使用 `Arc<DB>` 多线程共享 DB。
- 每个 worker 独立 RNG、独立 stats。
- 主线程每秒聚合并输出 interval delta。
- duration 和 operations 同时支持；任一达到即停止。

### 数据校验

小规模 correctness 模式：

- 单线程或受控并发。
- 保留 `BTreeMap` oracle。
- 每轮操作后抽样 get/scan 对比。

大规模 pressure 模式：

- 每个 key 的 value 编码为 `{key_id, version, checksum}`。
- 更新时递增线程本地或全局 version。
- read 时至少验证 value 内嵌 key_id 和 checksum。
- 对事务/冲突 workload 单独统计 conflict，不把预期 conflict 当错误。

## 需要补充的项目指标

优先级 P0：

- pressure binary 自身统计 op latency、ops/s、bytes/s、错误数。
- summary 中纳入现有 `MetricsSnapshot` 和 `level_file_counts`。
- 目录大小统计：WAL、SST、MANIFEST、total。

优先级 P1：

- DB metrics 增加 flush count/duration/bytes。
- DB metrics 增加 compaction count/duration。
- DB metrics 增加 write stall count/duration。
- recovery 路径统计 WAL replay bytes/records 和 MANIFEST edits。

优先级 P2：

- histogram 输出 HDR 兼容格式。
- perf/flamegraph 脚本。
- 和 RocksDB `db_bench` 同参数对比的 adapter。

## 结果判读

常见症状和可能原因：

- 写 p99 周期性尖刺：memtable flush、WAL sync、L0 slowdown/stop、manual compaction。
- read miss 很慢：Bloom filter 无效、block cache 太小、level 文件过多。
- read hit p99 升高：block cache 竞争、table/cache 打开成本、compaction 干扰。
- write amplification 持续升高：overwrite 产生多版本，compaction GC 不充分，snapshot 长时间存活。
- space amplification 持续升高：obsolete files 未删除、tombstone 未 GC、active snapshot 阻止 GC。
- reopen 时间增长：WAL 太大、MANIFEST edits 太多、table open 成本高。

## 落地顺序

1. 保留现有 `cargo bench --bench write_read`，作为 microbench 回归。
2. 新增 `src/bin/pressure.rs`，先支持 `fillseq`、`fillrandom`、`readrandom-hit`、`readrandom-miss`、`ycsb-a`。
3. 输出 interval text 和 summary JSON，包含 latency percentiles、ops/s、metrics、level file counts、目录大小。
4. 增加并发 worker、duration 模式、hotset 分布。
5. 增加 `readwhilewriting`、`overwrite`、`scan-short`、`ycsb-b/c/e/f`。
6. 增加 recovery workloads。
7. 补齐 DB 内部 flush/compaction/stall/recovery metrics。

第一版完成标准：

- 能用 release binary 对固定 workload 跑 5 分钟不崩溃。
- summary 能回答吞吐、p99、写放大、读放大、cache hit rate、L0/level 文件数。
- 小规模 correctness 模式能对比 oracle。
- 文档中推荐的快速本地回归命令可执行。
