# V4 VersionSet 与 MANIFEST 实现计划

> **给智能代理执行者：** 必须使用子技能 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐任务执行本计划。步骤使用复选框（`- [ ]`）语法便于跟踪。

**目标：** 通过 VersionSet、MANIFEST、CURRENT 与单调递增文件号持久化 table metadata 和引擎状态。

**架构：** 所有持久 metadata 变更都表示为 `VersionEdit`，先 append 到 MANIFEST，再在内存中可见。`CURRENT` 指向活跃 MANIFEST，更新时使用临时文件和 rename 语义。

**技术栈：** Rust 2024、标准库文件系统 API、V2 的 WAL/coding 工具、`cargo test`。

---

## 范围

本版本包含：

- 包含 `CURRENT`、`MANIFEST-000001`、WAL、SST 文件的 DB 目录布局。
- `FileMeta`、`Version`、`VersionEdit`、`VersionSet`。
- MANIFEST append 与 replay。
- 文件号分配器。
- flushed SSTable 与 active WAL 的 reopen recovery。

本版本不包含：

- 不实现 compaction。
- 不实现多个 MANIFEST 的轮转。
- 不删除 obsolete file。
- 不实现并发后台 worker。

## 文件划分

- 创建：`src/version/mod.rs`
- 创建：`src/version/version.rs`
- 创建：`src/version/edit.rs`
- 创建：`src/version/manifest.rs`
- 创建：`src/version/version_set.rs`
- 创建：`src/env/mod.rs`
- 创建：`src/env/fs.rs`
- 创建：`src/env/file.rs`
- 修改：`src/db.rs`
- 修改：`src/table/reader.rs`
- 修改：`src/lib.rs`
- 测试：`src/version/*` 模块测试和 reopen 集成测试

## 任务

- [ ] **步骤 1：定义文件 metadata 与 version**

  `FileMeta` 字段：

  - `number: u64`
  - `file_size: u64`
  - `smallest: InternalKey`
  - `largest: InternalKey`
  - `smallest_seq: u64`
  - `largest_seq: u64`

  `Version` 字段：

  - `l0_files: Vec<FileMeta>`
  - `levels: Vec<Vec<FileMeta>>`

- [ ] **步骤 2：定义 VersionEdit 编码**

  包含 variant：

  - `NextFileNumber(u64)`
  - `LastSequence(u64)`
  - `LogNumber(u64)`
  - `AddFile { level, meta }`
  - `DeleteFile { level, number }`

  使用 tag ID 和 varint 编码。为每个 variant 增加 roundtrip 测试。

- [ ] **步骤 3：实现 MANIFEST writer 与 reader**

  MANIFEST entry 复用 WAL-style record。`ManifestWriter::append(edit)` 对每个 metadata-changing edit 写入后执行 sync。`ManifestReader` replay 所有完整 record；遇到 checksum mismatch 返回 corruption。

- [ ] **步骤 4：实现 CURRENT 更新**

  增加 helper：

  ```rust
  fn set_current(db_path: &Path, manifest_name: &str) -> Result<()>
  ```

  它写入 `CURRENT.tmp`、sync 文件、rename 为 `CURRENT`，并在平台支持时 sync 目录。

- [ ] **步骤 5：实现 VersionSet**

  `VersionSet` 拥有：

  - `current: Arc<Version>`
  - `next_file_number`
  - `last_sequence`
  - `log_number`
  - `manifest_number`

  必需方法：

  - `new_empty(options.max_levels)`
  - `allocate_file_number()`
  - `log_and_apply(edit)`
  - `recover(db_path)`

- [ ] **步骤 6：让 flush 走 VersionSet**

  `DB::flush` 必须从 `VersionSet` 分配文件号，写 SST，append `AddFile(level=0, meta)`，然后才把新的 table reader 发布到 DB state。

- [ ] **步骤 7：接入 reopen recovery**

  `DB::open` 必须：

  - 如果存在 `CURRENT`，读取它。
  - replay MANIFEST 到 `VersionSet`。
  - 打开所有被引用的 SSTable。
  - replay active WAL。
  - 为后续写入创建新的 WAL number。

- [ ] **步骤 8：增加 manifest recovery 测试**

  测试：

  - `manifest_edit_roundtrip`
  - `current_points_to_manifest`
  - `reopen_recovers_flushed_sstables`
  - `reopen_preserves_next_file_number`
  - `manifest_rejects_corrupt_complete_record`

- [ ] **步骤 9：验证版本**

  运行：`cargo fmt`

  期望：命令成功退出。

  运行：`cargo test`

  期望：全部测试通过。

## 退出条件

- flushed SSTable 在进程 reopen 后仍可恢复。
- reopen 后文件号绝不回退。
- MANIFEST 是 table metadata 的事实来源。
- WAL recovery 与 SST recovery 协同工作，不会重复应用已 flush 的记录。

## 建议提交

```bash
git add src
git commit -m "feat: add VersionSet and MANIFEST recovery"
```
