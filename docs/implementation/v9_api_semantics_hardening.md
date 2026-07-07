# V9 API 语义与 Env 边界加固实现计划

> **给智能代理执行者：** 必须使用子技能 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐任务执行本计划。步骤使用复选框（`- [ ]`）语法便于跟踪。

**目标：** 补齐 V8 后验收发现的 API 语义缺口，让 `ReadOptions`、`Env`、读缓存行为和文档说明与当前公开能力一致。

**架构：** V9 不引入后台线程和新 compaction 策略，只在已有 `DB`、`SSTableReader`、`WalReader`、`TableCache` 和 `Env` 边界上做收口。读路径继续保持同步执行，但读文件打开与 block cache 行为必须可以被 options 控制并被测试验证。

**技术栈：** Rust 2024、标准库 IO trait、现有 `Env` trait、`cargo test`、`cargo clippy`、`cargo fmt`。

---

## 范围

本版本包含：

- 让 `ReadOptions.fill_cache` 对 `scan_opt` 生效。
- 扩展 `Env`，让 WAL/SSTable/MANIFEST 读路径也能通过可注入 Env 打开文件。
- 保留 `WalReader::open`、`SSTableReader::open` 等旧 API，并新增 `open_with_env` 供 DB 内部使用。
- 更新 `docs/usage.md`，移除已经修复的限制说明，并保留后台 worker、流式 iterator 等未实现能力说明。
- 增加针对 cache 语义和 Env 读路径的回归测试。

本版本不包含：

- 不实现后台 flush/compaction worker。
- 不实现流式 DB iterator。
- 不改变现有 `DB::scan` 返回 `Vec` 的公开 API。
- 不实现 remote/object-store Env。
- 不新增 SQL、column family、merge operator。

## 文件划分

- 修改：`src/env/fs.rs`
- 修改：`src/env/mod.rs`
- 修改：`src/wal/reader.rs`
- 修改：`src/version/manifest.rs`
- 修改：`src/table/reader.rs`
- 修改：`src/cache/table_cache.rs`
- 修改：`src/db.rs`
- 修改：`tests/v5_read_path_cache.rs`
- 修改：`tests/crash_recovery.rs`
- 修改：`docs/usage.md`
- 修改：`docs/implementation/README.md`

## 任务

- [x] **步骤 1：增加 `scan_opt` cache 语义回归测试**

  在 `tests/v5_read_path_cache.rs` 中增加测试：

  - 构造带 SSTable 的 DB。
  - 第一次 `scan_opt(..., ReadOptions { fill_cache: false, .. })` 后断言 block cache hit/miss 不增加。
  - 后续 `get` 仍应产生 miss，证明 scan 没有预热 cache。

- [x] **步骤 2：让 `scan_opt` 传递 `fill_cache`**

  修改 `SSTableReader::entries_with_cache` 签名，增加 `fill_cache: bool` 参数。`DB::scan_opt` 从 `ReadOptions.fill_cache` 传入该参数；compaction、事务冲突检测等内部路径继续使用 `true`，保持现有缓存行为。

- [x] **步骤 3：扩展 Env 读文件能力**

  在 `src/env/fs.rs` 增加：

  ```rust
  pub trait ReadableFile: Debug + Send {
      fn read(&mut self, dst: &mut [u8]) -> Result<usize>;
      fn read_exact(&mut self, dst: &mut [u8]) -> Result<()>;
      fn seek(&mut self, pos: SeekFrom) -> Result<u64>;
      fn len(&self) -> Result<u64>;
  }
  ```

  并在 `Env` 增加：

  ```rust
  fn open_readable(&self, path: &Path) -> Result<Box<dyn ReadableFile>>;
  ```

- [x] **步骤 4：把 WAL、MANIFEST、SSTable 读路径接入 Env**

  - `WalReader::open(path)` 保持默认 `FsEnv`。
  - 新增 `WalReader::open_with_env(env, path)`。
  - `ManifestReader::open(path)` 保持默认 `FsEnv`。
  - 新增 `ManifestReader::open_with_env(env, path)`。
  - `SSTableReader::open(path)` 保持默认 `FsEnv`。
  - 新增 `SSTableReader::open_with_env(env, path)`，reader 内部保存 `Arc<dyn Env>`，后续 data block 读取也通过该 env 打开文件。

- [x] **步骤 5：让 DB 和 TableCache 使用 Env 读路径**

  `TableCache` 持有 `Arc<dyn Env>`，`DB::open` 创建 table cache 时传入 `options.env.clone()`。`VersionSet::recover` 使用 `ManifestReader::open_with_env`。WAL recovery 使用 `WalReader::open_with_env`。

- [x] **步骤 6：增加 Env 读路径回归测试**

  在 `tests/crash_recovery.rs` 的 fault env 中实现 `open_readable` 计数。新增测试：

  - 写入并 flush 后 reopen。
  - 通过注入 env reopen，并执行 `get`。
  - 断言 env 的 readable open 次数大于 0，证明恢复和 SSTable 读取经过 Env。

- [x] **步骤 7：更新使用文档**

  修改 `docs/usage.md`：

  - 保留 `fill_cache` 会控制 get/scan 的说明。
  - 把 `env` 描述为读写路径都可注入的文件系统边界。
  - 保留 `verify_checksums`、`total_order_seek`、后台 worker、流式 iterator 等仍未实现限制。

- [x] **步骤 8：验证版本**

  运行：`cargo test --all-targets`

  期望：全部测试通过。

  运行：`cargo clippy --all-targets -- -D warnings`

  期望：无 warning。

  运行：`cargo fmt --check`

  期望：格式检查通过。

  运行：`git diff --check`

  期望：没有空白或冲突标记问题。

## 退出条件

- `ReadOptions.fill_cache = false` 对 `get_opt` 和 `scan_opt` 都生效。
- DB 内部 WAL recovery、MANIFEST recovery、SSTable metadata/data 读取都可以经过 `Options.env`。
- 旧的默认文件系统 API 保持可用。
- 新增回归测试覆盖 cache 语义和 Env 读路径。
- 文档中不再把已修复能力列为限制。

## 建议提交

```bash
git add docs src tests
git commit -m "feat: add V9 API semantics hardening"
```
