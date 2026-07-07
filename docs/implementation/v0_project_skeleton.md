# V0 工程骨架实现计划

> **给智能代理执行者：** 必须使用子技能 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐任务执行本计划。步骤使用复选框（`- [ ]`）语法便于跟踪。

**目标：** 把当前二进制占位工程改成库 crate，建立 TYLSMDB 的公开 API 形状、共享选项、错误处理、字节别名和可测试模块布局。

**架构：** 本版本创建后续路线都会依赖的 crate 边界。`DB` 是一个很薄的公开句柄，内部包住实现对象；真正存储行为在 V1 开始落地。

**技术栈：** Rust 2024、`cargo test`，除任务明确要求外只使用标准库。

---

## 范围

本版本包含：

- 创建 `src/lib.rs` 和设计文档中的顶层模块。
- 定义稳定的公开类型：`DB`、`Options`、`ReadOptions`、`WriteOptions`、`WriteBatch`、`Snapshot`、`TransactionOptions`。
- 定义全项目共用的 `Error` 与 `Result`。
- 保留 `src/main.rs` 作为很小的示例二进制，用来验证库接口可被调用。

本版本不包含：

- 不实现真实持久化。
- 不实现真实 memtable。
- 不实现 WAL、SSTable、MANIFEST、compaction、MVCC 或 cache 行为。

## 文件划分

- 创建：`src/lib.rs`
- 创建：`src/db.rs`
- 创建：`src/options.rs`
- 创建：`src/error.rs`
- 创建：`src/bytes.rs`
- 创建：`src/batch.rs`
- 创建：`src/snapshot.rs`
- 创建：`src/transaction.rs`
- 创建：`src/key/mod.rs`
- 创建：`src/key/internal_key.rs`
- 创建：`src/key/comparator.rs`
- 创建：`src/util/mod.rs`
- 创建：`src/util/coding.rs`
- 修改：`src/main.rs`
- 修改：`Cargo.toml`

## 任务

- [ ] **步骤 1：增加库入口**

  创建 `src/lib.rs` 并导出公开 API：

  ```rust
  pub mod batch;
  pub mod bytes;
  pub mod db;
  pub mod error;
  pub mod key;
  pub mod options;
  pub mod snapshot;
  pub mod transaction;
  pub mod util;

  pub use batch::{BatchRecord, WriteBatch};
  pub use db::DB;
  pub use error::{Error, Result};
  pub use options::{Options, ReadOptions, WalSyncMode, WriteOptions};
  pub use snapshot::Snapshot;
  pub use transaction::{Transaction, TransactionOptions};
  ```

- [ ] **步骤 2：增加通用错误类型**

  创建 `src/error.rs`：

  ```rust
  use std::fmt::{Display, Formatter};

  #[derive(Debug)]
  pub enum Error {
      InvalidArgument(String),
      Corruption(String),
      Io(std::io::Error),
      Closed,
      Unsupported(&'static str),
  }

  pub type Result<T> = std::result::Result<T, Error>;

  impl Display for Error {
      fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
          match self {
              Self::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
              Self::Corruption(msg) => write!(f, "corruption: {msg}"),
              Self::Io(err) => write!(f, "io error: {err}"),
              Self::Closed => write!(f, "database is closed"),
              Self::Unsupported(feature) => write!(f, "unsupported feature: {feature}"),
          }
      }
  }

  impl std::error::Error for Error {}

  impl From<std::io::Error> for Error {
      fn from(value: std::io::Error) -> Self {
          Self::Io(value)
      }
  }
  ```

- [ ] **步骤 3：增加选项结构**

  创建 `src/options.rs`，默认值与设计文档保持一致：

  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum WalSyncMode {
      Never,
      PerWrite,
  }

  #[derive(Debug, Clone)]
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
      pub bloom_false_positive_rate: f64,
      pub block_cache_capacity: usize,
      pub max_background_flushes: usize,
      pub max_background_compactions: usize,
      pub max_subcompactions: usize,
  }

  impl Default for Options {
      fn default() -> Self {
          Self {
              create_if_missing: true,
              error_if_exists: false,
              memtable_size: 4 * 1024 * 1024,
              max_immutable_memtables: 3,
              block_size: 4 * 1024,
              target_file_size_base: 64 * 1024 * 1024,
              max_levels: 7,
              level0_file_num_compaction_trigger: 4,
              level0_slowdown_writes_trigger: 12,
              level0_stop_writes_trigger: 20,
              max_bytes_for_level_base: 256 * 1024 * 1024,
              max_bytes_for_level_multiplier: 10.0,
              wal_enabled: true,
              wal_sync: WalSyncMode::Never,
              bloom_false_positive_rate: 0.01,
              block_cache_capacity: 64 * 1024 * 1024,
              max_background_flushes: 1,
              max_background_compactions: 1,
              max_subcompactions: 1,
          }
      }
  }

  #[derive(Debug, Clone, Copy)]
  pub struct WriteOptions {
      pub sync: bool,
      pub disable_wal: bool,
  }

  impl Default for WriteOptions {
      fn default() -> Self {
          Self {
              sync: false,
              disable_wal: false,
          }
      }
  }

  #[derive(Debug, Clone)]
  pub struct ReadOptions {
      pub verify_checksums: bool,
      pub fill_cache: bool,
      pub total_order_seek: bool,
  }

  impl Default for ReadOptions {
      fn default() -> Self {
          Self {
              verify_checksums: true,
              fill_cache: true,
              total_order_seek: false,
          }
      }
  }
  ```

- [ ] **步骤 4：增加字节与 batch 类型**

  创建 `src/bytes.rs`：

  ```rust
  pub type Bytes = Vec<u8>;
  pub type UserKey = [u8];
  pub type UserKeyBuf = Vec<u8>;
  ```

  创建 `src/batch.rs`：

  ```rust
  use crate::bytes::Bytes;

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum BatchRecord {
      Put { key: Bytes, value: Bytes },
      Delete { key: Bytes },
  }

  #[derive(Debug, Clone, Default, PartialEq, Eq)]
  pub struct WriteBatch {
      records: Vec<BatchRecord>,
  }

  impl WriteBatch {
      pub fn new() -> Self {
          Self::default()
      }

      pub fn put(&mut self, key: impl Into<Bytes>, value: impl Into<Bytes>) {
          self.records.push(BatchRecord::Put {
              key: key.into(),
              value: value.into(),
          });
      }

      pub fn delete(&mut self, key: impl Into<Bytes>) {
          self.records.push(BatchRecord::Delete { key: key.into() });
      }

      pub fn records(&self) -> &[BatchRecord] {
          &self.records
      }

      pub fn is_empty(&self) -> bool {
          self.records.is_empty()
      }
  }
  ```

- [ ] **步骤 5：增加 DB API 骨架**

  创建 `src/db.rs`，方法签名先编译通过；尚未实现的行为返回 `Unsupported`：

  ```rust
  use std::ops::Bound;
  use std::path::{Path, PathBuf};
  use std::sync::Arc;

  use crate::batch::WriteBatch;
  use crate::bytes::Bytes;
  use crate::error::{Error, Result};
  use crate::options::{Options, ReadOptions, WriteOptions};
  use crate::snapshot::Snapshot;
  use crate::transaction::{Transaction, TransactionOptions};

  #[derive(Debug, Clone)]
  pub struct DB {
      inner: Arc<DBInner>,
  }

  #[derive(Debug)]
  struct DBInner {
      path: PathBuf,
      options: Options,
  }

  impl DB {
      pub fn open(path: impl AsRef<Path>, options: Options) -> Result<Self> {
          Ok(Self {
              inner: Arc::new(DBInner {
                  path: path.as_ref().to_path_buf(),
                  options,
              }),
          })
      }

      pub fn close(&self) -> Result<()> {
          Ok(())
      }

      pub fn path(&self) -> &Path {
          &self.inner.path
      }

      pub fn options(&self) -> &Options {
          &self.inner.options
      }

      pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
          let mut batch = WriteBatch::new();
          batch.put(key.to_vec(), value.to_vec());
          self.write(batch, WriteOptions::default())
      }

      pub fn delete(&self, key: &[u8]) -> Result<()> {
          let mut batch = WriteBatch::new();
          batch.delete(key.to_vec());
          self.write(batch, WriteOptions::default())
      }

      pub fn write(&self, _batch: WriteBatch, _opts: WriteOptions) -> Result<()> {
          Err(Error::Unsupported("persistent writes arrive in V1"))
      }

      pub fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
          self.get_opt(key, ReadOptions::default())
      }

      pub fn get_opt(&self, _key: &[u8], _opts: ReadOptions) -> Result<Option<Bytes>> {
          Err(Error::Unsupported("point reads arrive in V1"))
      }

      pub fn scan(&self, _lower: Bound<&[u8]>, _upper: Bound<&[u8]>) -> Result<Vec<(Bytes, Bytes)>> {
          Err(Error::Unsupported("range scans arrive in V1"))
      }

      pub fn snapshot(&self) -> Snapshot {
          Snapshot::new(0)
      }

      pub fn transaction(&self, _opts: TransactionOptions) -> Result<Transaction> {
          Err(Error::Unsupported("transactions arrive in V7"))
      }

      pub fn flush(&self) -> Result<()> {
          Err(Error::Unsupported("flush arrives in V3"))
      }

      pub fn compact_range(&self, _lower: Bound<&[u8]>, _upper: Bound<&[u8]>) -> Result<()> {
          Err(Error::Unsupported("compaction arrives in V6"))
      }

      pub fn sync_wal(&self) -> Result<()> {
          Err(Error::Unsupported("WAL arrives in V2"))
      }
  }
  ```

- [ ] **步骤 6：增加 snapshot 与 transaction 占位类型**

  创建 `src/snapshot.rs`：

  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub struct Snapshot {
      read_seq: u64,
  }

  impl Snapshot {
      pub(crate) fn new(read_seq: u64) -> Self {
          Self { read_seq }
      }

      pub fn read_seq(&self) -> u64 {
          self.read_seq
      }
  }
  ```

  创建 `src/transaction.rs`：

  ```rust
  #[derive(Debug, Clone, Default)]
  pub struct TransactionOptions {
      pub read_only: bool,
  }

  #[derive(Debug)]
  pub struct Transaction {
      read_seq: u64,
  }

  impl Transaction {
      pub(crate) fn new(read_seq: u64) -> Self {
          Self { read_seq }
      }

      pub fn read_seq(&self) -> u64 {
          self.read_seq
      }
  }
  ```

- [ ] **步骤 7：增加 key 与 coding 模块空壳**

  创建 `src/key/mod.rs`：

  ```rust
  pub mod comparator;
  pub mod internal_key;
  ```

  创建 `src/key/internal_key.rs`：

  ```rust
  pub type SequenceNumber = u64;
  ```

  创建 `src/key/comparator.rs`：

  ```rust
  use std::cmp::Ordering;

  pub fn compare_user_key(left: &[u8], right: &[u8]) -> Ordering {
      left.cmp(right)
  }
  ```

  创建 `src/util/mod.rs`：

  ```rust
  pub mod coding;
  ```

  创建 `src/util/coding.rs`：

  ```rust
  use crate::error::{Error, Result};

  pub fn require_non_empty(input: &[u8], context: &'static str) -> Result<()> {
      if input.is_empty() {
          return Err(Error::InvalidArgument(format!("{context} must not be empty")));
      }
      Ok(())
  }
  ```

- [ ] **步骤 8：更新二进制入口**

  修改 `src/main.rs`：

  ```rust
  use tylsmdb::{DB, Options};

  fn main() -> tylsmdb::Result<()> {
      let db = DB::open("target/tylsmdb-example", Options::default())?;
      println!("opened TYLSMDB at {}", db.path().display());
      Ok(())
  }
  ```

- [ ] **步骤 9：验证骨架**

  运行：`cargo fmt`

  期望：命令成功退出。

  运行：`cargo test`

  期望：crate 能编译，全部测试通过。

## 退出条件

- `cargo test` 通过。
- `DB::open`、`Options::default`、`WriteBatch::new`、`Snapshot::read_seq` 能被外部 crate 调用。
- 除骨架 API 外，不声称已经具备任何存储能力。

## 建议提交

```bash
git add Cargo.toml src
git commit -m "chore: add TYLSMDB library skeleton"
```
