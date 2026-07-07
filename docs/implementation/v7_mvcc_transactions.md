# V7 MVCC 与事务实现计划

> **给智能代理执行者：** 必须使用子技能 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐任务执行本计划。步骤使用复选框（`- [ ]`）语法便于跟踪。

**目标：** 增加 snapshot read、水位线、乐观事务和 Serializable Snapshot Isolation 校验。

**架构：** Snapshot 在 drop 前 pin 住一个 read sequence。Transaction 缓冲写入、记录 read set 与 write set，在 commit 时做冲突校验，然后以新的 commit sequence 写入一个原子 batch。

**技术栈：** Rust 2024、前面版本建立的 sequence-numbered storage、`cargo test`。

---

## 范围

本版本包含：

- 可通过 `ReadOptions` 使用的 read snapshot。
- 活跃 snapshot 的 watermark 跟踪。
- 带 `get`、`put`、`delete`、`scan`、`commit`、`rollback` 的 Transaction API。
- 乐观 write-write 与 read-write conflict detection。
- scanned range 的 SSI 校验。

本版本不包含：

- 不实现悲观锁。
- 不实现独立于 WriteBatch 的持久 transaction intent record。
- 不实现跨进程事务。

## 文件划分

- 创建：`src/mvcc/mod.rs`
- 创建：`src/mvcc/snapshot.rs`
- 创建：`src/mvcc/watermark.rs`
- 创建：`src/mvcc/transaction.rs`
- 创建：`src/mvcc/conflict.rs`
- 修改：`src/snapshot.rs`
- 修改：`src/transaction.rs`
- 修改：`src/options.rs`
- 修改：`src/db.rs`
- 修改：`src/compact/executor.rs`
- 修改：`src/lib.rs`
- 测试：`src/mvcc/*` 模块测试和 transaction 集成测试

## 任务

- [ ] **步骤 1：为 ReadOptions 扩展 snapshot**

  增加：

  ```rust
  pub snapshot: Option<Snapshot>
  ```

  `DB::get_opt` 和 `DB::scan_opt` 在存在 snapshot 时使用 `snapshot.read_seq()`，否则使用当前 `last_sequence`。

- [ ] **步骤 2：实现 watermark**

  `Watermark` 用 multiset 跟踪活跃 read sequence，并暴露：

  - `add(seq)`
  - `remove(seq)`
  - `oldest() -> Option<u64>`

  只有当旧版本早于 oldest active snapshot，且被更新版本隐藏时，compaction 才能丢弃它。

- [ ] **步骤 3：让 Snapshot 具备 RAII 语义**

  `DB::snapshot()` 把 `last_sequence` 注册到 watermark。Drop snapshot 时注销。Clone snapshot 时增加注册计数，每个 clone drop 时减少计数。

- [ ] **步骤 4：定义 transaction state**

  Transaction 字段：

  - `db: DB`
  - `read_seq`
  - `writes: WriteBatch`
  - `read_keys: BTreeSet<Vec<u8>>`
  - `read_ranges: Vec<(Bound<Vec<u8>>, Bound<Vec<u8>>)>`
  - `closed: bool`

- [ ] **步骤 5：实现 transaction reads 与 writes**

  Transaction read 先检查本地 writes，再以 `read_seq` 读取 DB。Writes 只追加到本地 batch，commit 前不可见。

- [ ] **步骤 6：实现 conflict detection**

  Commit 时：

  - 如果 `read_keys` 中任意 key 在 `read_seq` 后发生变化，拒绝 commit。
  - 如果 `writes` 中任意 key 在 `read_seq` 后被其他事务修改，拒绝 commit。
  - 如果 `read_seq` 后插入或删除的任意 key 落在记录过的 scanned range 内，拒绝 commit。

  增加 `Error::TransactionConflict(String)`。

- [ ] **步骤 7：实现 commit 与 rollback**

  `commit` 校验冲突，并通过 `DB::write` 原子写入缓冲 batch。`rollback` 标记 transaction closed 并丢弃缓冲写入。

- [ ] **步骤 8：更新 compaction drop 规则**

  Compaction 使用 watermark，保留仍可能被 active snapshot 或 transaction 看到的版本。

- [ ] **步骤 9：增加 MVCC 测试**

  测试：

  - `snapshot_keeps_old_value_after_update`
  - `snapshot_drop_allows_old_version_gc`
  - `transaction_commit_applies_batch_atomically`
  - `write_write_conflict_is_rejected`
  - `read_write_conflict_is_rejected`
  - `range_phantom_conflict_is_rejected`
  - `rollback_discards_writes`

- [ ] **步骤 10：验证版本**

  运行：`cargo fmt`

  期望：命令成功退出。

  运行：`cargo test`

  期望：全部测试通过。

## 退出条件

- Snapshot 提供稳定的历史读取。
- Transaction commit 是原子 WriteBatch 操作。
- 冲突的乐观事务返回类型化 conflict error。
- Compaction 不会丢弃 active snapshot 可见的版本。

## 建议提交

```bash
git add src
git commit -m "feat: add MVCC snapshots and optimistic transactions"
```
