# V1 内存引擎实现计划

> **给智能代理执行者：** 必须使用子技能 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐任务执行本计划。步骤使用复选框（`- [ ]`）语法便于跟踪。

**目标：** 交付一个可工作的内存 KV 引擎，支持 sequence number、tombstone、点查、批量写和有序范围扫描。

**架构：** `DB` 拥有一个 mutable `MemTable`，底层使用 `BTreeMap<InternalKey, ValueRecord>`。公开读请求会获取一致的 read sequence，并把内部多版本记录过滤成用户可见结果。

**技术栈：** Rust 2024、标准库 `BTreeMap`、`cargo test`。

---

## 范围

本版本包含：

- 实现 `InternalKey` 排序：user key 升序、sequence 降序、value type 稳定排序。
- 实现 `MemTable` 的 `put`、`delete`、`get` 和范围迭代。
- 让 `DB::put`、`DB::delete`、`DB::write`、`DB::get`、`DB::scan` 在内存中工作。
- 增加 tombstone、batch sequence 分配、scan 边界测试。

本版本不包含：

- 不实现 WAL 持久化。
- 不实现 immutable memtable。
- 不实现真正 snapshot，只支持“当前 sequence”读取。
- 不做并发优化，只使用简单 `RwLock`。

## 文件划分

- 修改：`src/key/internal_key.rs`
- 修改：`src/key/comparator.rs`
- 创建：`src/memtable/mod.rs`
- 创建：`src/memtable/btree.rs`
- 修改：`src/db.rs`
- 修改：`src/lib.rs`
- 测试：`src/key/internal_key.rs`、`src/memtable/btree.rs`、`src/db.rs` 内的模块测试

## 任务

- [ ] **步骤 1：实现 internal key 模型**

  定义：

  ```rust
  pub type SequenceNumber = u64;

  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum ValueType {
      Put = 1,
      Delete = 2,
  }

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct InternalKey {
      user_key: Vec<u8>,
      sequence: SequenceNumber,
      value_type: ValueType,
  }
  ```

  实现 `Ord`，确保 `(b"a", 9, Put)` 排在 `(b"a", 8, Put)` 前面，`(b"a", 8, Put)` 排在 `(b"b", 100, Put)` 前面。

- [ ] **步骤 2：测试 key 排序**

  增加测试：

  ```rust
  #[test]
  fn internal_key_orders_user_key_asc_and_sequence_desc() {
      let mut keys = vec![
          InternalKey::new(b"a".to_vec(), 7, ValueType::Put),
          InternalKey::new(b"a".to_vec(), 9, ValueType::Put),
          InternalKey::new(b"b".to_vec(), 1, ValueType::Put),
      ];
      keys.sort();
      assert_eq!(keys[0].sequence(), 9);
      assert_eq!(keys[1].sequence(), 7);
      assert_eq!(keys[2].user_key(), b"b");
  }
  ```

  运行：`cargo test key::internal_key`

  期望：排序测试通过。

- [ ] **步骤 3：实现 BTree memtable**

  增加 `MemTable`：

  ```rust
  pub enum ValueRecord {
      Put(Vec<u8>),
      Delete,
  }

  pub struct MemTable {
      map: BTreeMap<InternalKey, ValueRecord>,
      approximate_size: usize,
  }
  ```

  必须实现的方法：

  - `put(seq, key, value)`
  - `delete(seq, key)`
  - `get(key, read_seq) -> Option<ValueRecord>`
  - `scan(lower, upper, read_seq) -> Vec<(Vec<u8>, Vec<u8>)>`
  - `approximate_size() -> usize`

- [ ] **步骤 4：测试 memtable 可见性**

  覆盖以下场景：

  - 最新可见值胜出。
  - tombstone 隐藏旧值。
  - 较低的 `read_seq` 能看到旧版本。
  - scan 返回排序后的唯一 user key。

  运行：`cargo test memtable`

  期望：全部 memtable 测试通过。

- [ ] **步骤 5：接入 DB 状态**

  把 V0 的骨架内部状态替换为：

  ```rust
  struct DBInner {
      path: PathBuf,
      options: Options,
      state: RwLock<DBState>,
  }

  struct DBState {
      mutable: MemTable,
      last_sequence: SequenceNumber,
      closed: bool,
  }
  ```

  `DB::write` 对空 batch 返回 `Ok(())`；非空 batch 为每条记录分配一个 sequence，在写锁内应用所有记录，并发布最终 sequence。

- [ ] **步骤 6：测试公开内存 API**

  增加测试：

  - `put_then_get_returns_value`
  - `delete_hides_existing_value`
  - `write_batch_is_applied_in_order`
  - `scan_respects_included_excluded_bounds`

  运行：`cargo test db::`

  期望：公开 API 测试通过。

- [ ] **步骤 7：验证版本**

  运行：`cargo fmt`

  期望：命令成功退出。

  运行：`cargo test`

  期望：全部测试通过。

## 退出条件

- `DB` 能作为进程内内存 KV 存储使用。
- 删除在内部使用 tombstone，而不是直接抹掉全部历史。
- 公开 scan 返回有序 user key，且不会返回重复版本。
- V0 的公开行为保持源码兼容。

## 建议提交

```bash
git add src
git commit -m "feat: add in-memory LSM write and read path"
```
