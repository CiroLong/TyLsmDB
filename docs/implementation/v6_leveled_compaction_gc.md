# V6 分层压缩合并与 GC 实现计划

> **给智能代理执行者：** 必须使用子技能 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐任务执行本计划。步骤使用复选框（`- [ ]`）语法便于跟踪。

**目标：** 增加 leveled compaction、obsolete file 清理、tombstone/version GC 和写入限流。

**架构：** Compaction picker 从当前 Version 计算 score。Compaction executor 合并选中的输入 table，生成输出 SSTable，写 MANIFEST edit，发布新 Version，并且只在 reader 不再引用旧文件后调度 obsolete file 删除。

**技术栈：** Rust 2024、V5 的 iterator 框架、V4 的 VersionSet、`cargo test`。

---

## 范围

本版本包含：

- L0 到 L1、Ln 到 Ln+1 的 leveled compaction。
- 手动 `compact_range`。
- 安全场景下的 trivial move。
- 丢弃 obsolete version 和 tombstone。
- Obsolete file 跟踪与删除。
- L0 写入 slowdown 与 stop 阈值。

本版本不包含：

- 不实现 subcompaction。
- 不实现 rate limiter。
- 不实现 universal 或 tiered compaction。
- 不实现 compaction filter。

## 文件划分

- 创建：`src/compact/mod.rs`
- 创建：`src/compact/picker.rs`
- 创建：`src/compact/task.rs`
- 创建：`src/compact/leveled.rs`
- 创建：`src/compact/executor.rs`
- 修改：`src/version/version.rs`
- 修改：`src/version/version_set.rs`
- 修改：`src/db.rs`
- 修改：`src/table/builder.rs`
- 修改：`src/lib.rs`
- 测试：`src/compact/*` 模块测试和 DB compaction 集成测试

## 任务

- [ ] **步骤 1：定义 compaction task 类型**

  `CompactionTask` 字段：

  - `input_level`
  - `output_level`
  - `input_files`
  - `overlap_files`
  - `smallest_user_key`
  - `largest_user_key`
  - `is_manual`

- [ ] **步骤 2：实现 level scoring**

  Score 规则：

  - L0 score 为 `l0_file_count / level0_file_num_compaction_trigger`。
  - L1+ score 为 `level_bytes / max_bytes_for_level(level)`。
  - 选择 score 大于等于 `1.0` 的最高分 level。

- [ ] **步骤 3：实现 compaction picker**

  L0 picker 选择足够老的 L0 文件以降低 overlap 压力，并包含所有重叠的 L1 文件。L1+ picker 选择 input level 的一个文件，以及 output level 中所有重叠文件。

- [ ] **步骤 4：实现 trivial move**

  如果非 L0 input file 在 output level 没有 overlap，则只通过 MANIFEST edit 把 metadata 从 input level 移到 output level，不重写 SSTable。

- [ ] **步骤 5：实现 compaction executor**

  Executor：

  - 打开输入 iterator。
  - 用 V5 iterator 合并输入。
  - 在没有 snapshot 需要时丢弃被隐藏版本。
  - 当 tombstone 不可能再隐藏低层数据时丢弃 tombstone。
  - 按 `target_file_size_base` 限制输出文件大小。
  - 应用一个 VersionEdit：删除 inputs 并添加 outputs。

- [ ] **步骤 6：增加 obsolete file 管理**

  Version 发布后跟踪 obsolete file number。只有当没有 live DB state 引用旧 Version 时才删除文件。删除错误需要记录下来，并在下一次 cleanup pass 重试。

- [ ] **步骤 7：实现写入限流**

  写入前：

  - 如果 L0 文件数超过 `level0_stop_writes_trigger`，阻塞直到 compaction 降低文件数。
  - 如果 L0 文件数超过 `level0_slowdown_writes_trigger`，先 sleep 一段配置好的小延迟再接受写入。

- [ ] **步骤 8：实现手动 compaction**

  `DB::compact_range(lower, upper)` 反复 pick 并执行与 user range 重叠的 task，直到没有匹配 compaction。

- [ ] **步骤 9：增加 compaction 测试**

  测试：

  - `l0_compaction_preserves_newest_values`
  - `leveled_compaction_outputs_non_overlapping_ranges`
  - `tombstone_drops_when_no_lower_level_overlap`
  - `manual_compact_range_reduces_overlapping_files`
  - `obsolete_files_are_deleted_after_publish`
  - `write_stall_blocks_when_l0_stop_threshold_is_reached`

- [ ] **步骤 10：验证版本**

  运行：`cargo fmt`

  期望：命令成功退出。

  运行：`cargo test`

  期望：全部测试通过。

## 退出条件

- Compaction 保持点查和 scan 正确性。
- L1+ 文件保持排序且不重叠。
- Obsolete SST file 在不可达后被删除。
- L0 压力能按 options 触发 slowdown 或 stop。

## 建议提交

```bash
git add src
git commit -m "feat: add leveled compaction and file GC"
```
