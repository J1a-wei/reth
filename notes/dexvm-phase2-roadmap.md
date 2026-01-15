# DexVM 第二阶段实施路线图

**更新日期**: 2026-01-14
**当前状态**: 第一阶段已完成，开始第二阶段
**架构策略**: DexVM为主 + EVM预留（暂不启用）

---

## 📋 第一阶段回顾

### ✅ 已完成内容

1. **核心组件**
   - ✅ 订单簿引擎（纳秒级性能）
   - ✅ 撮合引擎（价格-时间优先）
   - ✅ 状态管理（账户、余额、nonce）
   - ✅ 区块执行器
   - ✅ 单节点共识
   - ✅ 交易池
   - ✅ RLP编码/解码

2. **性能表现**
   - ✅ 订单簿核心: **119-220ns** (世界级)
   - ⚠️ 端到端TPS: **3,400** (待优化，目标200K)
   - ✅ 无全局锁架构
   - ✅ 并发安全设计

3. **已知瓶颈**
   - 签名验证（当前使用测试签名）
   - 顺序执行（未并行化）
   - 状态锁竞争
   - 无持久化存储

---

## 🎯 第二阶段目标

### 核心目标

1. **持久化存储**
   - 集成MDBX数据库
   - 状态快照和恢复
   - 历史数据查询

2. **性能优化**
   - 实现真实签名验证
   - 交易并行执行
   - 达到 **200K TPS** 目标

3. **EVM预留**
   - 保留EVM接口但不启用
   - 设计双VM状态隔离
   - 为未来集成做准备

4. **功能完善**
   - 完成取消订单
   - 改进查询API
   - 增强错误处理

---

## 📐 架构设计：DexVM为主 + EVM预留

### 整体架构

```
┌────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
│                                                              │
│  ┌────────────────┐              ┌──────────────────┐      │
│  │  DexVM 交易    │              │   EVM 交易       │      │
│  │  (主要业务)    │              │  (预留，不启用)  │      │
│  └────────┬───────┘              └───────┬──────────┘      │
└───────────┼─────────────────────────────┼──────────────────┘
            │                             │
            ▼                             ▼
┌───────────────────────────────────────────────────────────┐
│                    Execution Layer                         │
│                                                             │
│  ┌──────────────────────────────────────────────────┐     │
│  │           DexVmBlockExecutor (核心执行器)        │     │
│  │  - 处理DexVM交易                                  │     │
│  │  - 调用订单簿引擎                                │     │
│  │  - 并行执行优化                                   │     │
│  └──────────────────┬───────────────────────────────┘     │
│                     │                                      │
│  ┌──────────────────▼───────────────────────────────┐     │
│  │        EvmBlockExecutor (预留接口)               │     │
│  │  - 接口定义完整                                   │     │
│  │  - 暂时返回"未启用"错误                          │     │
│  │  - 保持与DexVM状态隔离                           │     │
│  └──────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────┘
            │
            ▼
┌─────────────────────────────────────────────────────────────┐
│                    State Layer                               │
│                                                               │
│  ┌────────────────┐  ┌─────────────┐  ┌─────────────────┐  │
│  │  DexVM State   │  │ Shared Nonce│  │  EVM State      │  │
│  │  - 订单簿      │  │ - 账户nonce │  │  (预留，空结构) │  │
│  │  - 持仓        │  │ - 统一管理  │  │                 │  │
│  │  - 保证金      │  │             │  │                 │  │
│  └────────────────┘  └─────────────┘  └─────────────────┘  │
└─────────────────────────────────────────────────────────────┘
            │
            ▼
┌─────────────────────────────────────────────────────────────┐
│               Storage Layer (Reth MDBX)                      │
│                                                               │
│  ┌───────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │  DexVM Tables │  │  Shared      │  │  EVM Tables  │     │
│  │  - Orders     │  │  - Accounts  │  │  (预留)      │     │
│  │  - Positions  │  │  - Balances  │  │              │     │
│  │  - Trades     │  │              │  │              │     │
│  └───────────────┘  └──────────────┘  └──────────────┘     │
└─────────────────────────────────────────────────────────────┘
```

### 关键设计原则

1. **DexVM优先**
   - 所有DEX业务逻辑在DexVM中执行
   - 订单簿、撮合、持仓管理完全由DexVM处理
   - 性能优化集中在DexVM执行路径

2. **EVM预留设计**
   - 定义完整的EVM执行器接口
   - 实现空的EVM状态结构
   - 数据库表结构预留但不填充
   - 接口调用返回"功能未启用"

3. **状态隔离**
   - DexVM和EVM状态完全隔离
   - 仅共享账户nonce（统一序列号）
   - 未来可通过预编译桥接

4. **存储分离**
   - DexVM表：Orders, Positions, Trades, Orderbooks
   - 共享表：Accounts, Balances, Nonces
   - EVM表：Code, Storage, Logs (预留)

---

## 🏗️ 第二阶段实施计划

### 任务2.1: 持久化存储集成 (核心)

**时间估算**: 2-3周

#### 目标
将内存状态持久化到MDBX数据库，支持节点重启后恢复。

#### 实施步骤

**2.1.1 定义数据库表结构**

```rust
// crates/dexvm/storage/src/tables.rs

use reth_db::table;
use alloy_primitives::{Address, B256, U256};
use reth_dexvm_primitives::*;

/// DexVM专用表定义

/// 账户表（共享）
table!(
    /// Account: address -> (nonce, balance)
    DexAccounts<Address, DexAccount> = "DexVM_Accounts"
);

/// 订单表
table!(
    /// Orders: order_id -> Order
    DexOrders<B256, Order> = "DexVM_Orders"
);

/// 订单索引：用户 -> 订单ID列表
table!(
    /// UserOrders: (address, order_id) -> ()
    DexUserOrders<(Address, B256), ()> = "DexVM_UserOrders"
);

/// 持仓表
table!(
    /// Positions: (address, trading_pair) -> Position
    DexPositions<(Address, TradingPair), Position> = "DexVM_Positions"
);

/// 成交记录表
table!(
    /// Trades: trade_id -> Trade
    DexTrades<B256, Trade> = "DexVM_Trades"
);

/// 区块元数据表
table!(
    /// BlockMeta: block_number -> BlockMeta
    DexBlockMeta<u64, DexBlockMetadata> = "DexVM_BlockMeta"
);

/// EVM预留表（不启用）
table!(
    /// EvmCode: code_hash -> bytecode (预留)
    EvmCode<B256, Vec<u8>> = "EVM_Code"
);

table!(
    /// EvmStorage: (address, slot) -> value (预留)
    EvmStorage<(Address, U256), U256> = "EVM_Storage"
);

// 辅助结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexAccount {
    pub nonce: u64,
    pub balances: HashMap<Address, U256>,        // token -> available
    pub frozen_balances: HashMap<Address, U256>, // token -> frozen
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub size: i128,           // 正数=多仓，负数=空仓
    pub entry_price: U256,
    pub margin: U256,
    pub unrealized_pnl: i128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexBlockMetadata {
    pub block_number: u64,
    pub timestamp: u64,
    pub tx_count: usize,
    pub gas_used: u64,
    pub state_root: B256,
}
```

**2.1.2 实现存储Provider**

```rust
// crates/dexvm/storage/src/provider.rs

use reth_db::{Database, DatabaseError};
use reth_db::transaction::DbTx;

pub struct DexStorageProvider<DB: Database> {
    db: DB,
}

impl<DB: Database> DexStorageProvider<DB> {
    pub fn new(db: DB) -> Self {
        Self { db }
    }

    /// 获取账户
    pub fn get_account(&self, address: &Address) -> Result<Option<DexAccount>, DatabaseError> {
        let tx = self.db.tx()?;
        tx.get::<DexAccounts>(address)
    }

    /// 保存账户
    pub fn save_account(&self, address: Address, account: DexAccount) -> Result<(), DatabaseError> {
        let tx = self.db.tx_mut()?;
        tx.put::<DexAccounts>(address, account)?;
        tx.commit()
    }

    /// 获取订单
    pub fn get_order(&self, order_id: &B256) -> Result<Option<Order>, DatabaseError> {
        let tx = self.db.tx()?;
        tx.get::<DexOrders>(order_id)
    }

    /// 保存订单
    pub fn save_order(&self, order: Order) -> Result<(), DatabaseError> {
        let tx = self.db.tx_mut()?;

        // 保存订单
        tx.put::<DexOrders>(order.id, order.clone())?;

        // 更新用户订单索引
        tx.put::<DexUserOrders>((order.maker, order.id), ())?;

        tx.commit()
    }

    /// 获取用户所有订单
    pub fn get_user_orders(&self, user: &Address) -> Result<Vec<Order>, DatabaseError> {
        let tx = self.db.tx()?;
        let mut orders = Vec::new();

        // 遍历用户订单索引
        let cursor = tx.cursor_read::<DexUserOrders>()?;
        for entry in cursor.walk_range((user.clone(), B256::ZERO)..=(user.clone(), B256::repeat_byte(0xff)))? {
            let ((_, order_id), _) = entry?;
            if let Some(order) = tx.get::<DexOrders>(&order_id)? {
                orders.push(order);
            }
        }

        Ok(orders)
    }

    /// 删除订单（取消订单时）
    pub fn delete_order(&self, order: &Order) -> Result<(), DatabaseError> {
        let tx = self.db.tx_mut()?;

        tx.delete::<DexOrders>(order.id, None)?;
        tx.delete::<DexUserOrders>((order.maker, order.id), None)?;

        tx.commit()
    }

    /// 保存成交记录
    pub fn save_trade(&self, trade: Trade) -> Result<(), DatabaseError> {
        let tx = self.db.tx_mut()?;
        tx.put::<DexTrades>(trade.id, trade)?;
        tx.commit()
    }

    /// 获取持仓
    pub fn get_position(&self, user: &Address, pair: &TradingPair) -> Result<Option<Position>, DatabaseError> {
        let tx = self.db.tx()?;
        tx.get::<DexPositions>(&(user.clone(), pair.clone()))
    }

    /// 保存持仓
    pub fn save_position(&self, user: Address, pair: TradingPair, position: Position) -> Result<(), DatabaseError> {
        let tx = self.db.tx_mut()?;
        tx.put::<DexPositions>((user, pair), position)?;
        tx.commit()
    }

    /// 保存区块元数据
    pub fn save_block_meta(&self, meta: DexBlockMetadata) -> Result<(), DatabaseError> {
        let tx = self.db.tx_mut()?;
        tx.put::<DexBlockMeta>(meta.block_number, meta)?;
        tx.commit()
    }

    /// 获取最新区块号
    pub fn get_latest_block_number(&self) -> Result<Option<u64>, DatabaseError> {
        let tx = self.db.tx()?;
        let cursor = tx.cursor_read::<DexBlockMeta>()?;
        Ok(cursor.last()?.map(|(block_num, _)| block_num))
    }
}
```

**2.1.3 集成到State**

```rust
// crates/dexvm/core/src/state.rs (更新)

pub struct DexVmState<DB: Database> {
    // 内存状态（热数据）
    accounts: Arc<RwLock<DexAccountState>>,
    matching_engine: Arc<MatchingEngine>,
    current_timestamp: Arc<RwLock<u64>>,

    // 持久化层
    storage: Arc<DexStorageProvider<DB>>,

    // 写缓存（批量提交优化）
    write_buffer: Arc<RwLock<WriteBuffer>>,
}

struct WriteBuffer {
    accounts: HashMap<Address, DexAccount>,
    orders: HashMap<B256, Order>,
    positions: HashMap<(Address, TradingPair), Position>,
    trades: Vec<Trade>,
}

impl<DB: Database> DexVmState<DB> {
    pub fn new(storage: Arc<DexStorageProvider<DB>>) -> Self {
        Self {
            accounts: Arc::new(RwLock::new(DexAccountState::new())),
            matching_engine: Arc::new(MatchingEngine::new()),
            current_timestamp: Arc::new(RwLock::new(0)),
            storage,
            write_buffer: Arc::new(RwLock::new(WriteBuffer::default())),
        }
    }

    /// 从数据库恢复状态
    pub fn recover_from_storage(&self) -> Result<(), DexVmError> {
        info!("Recovering DexVM state from storage...");

        // 1. 恢复账户
        // TODO: 实现账户迭代和恢复

        // 2. 恢复活跃订单到订单簿
        // TODO: 加载所有未完成订单

        // 3. 恢复持仓
        // TODO: 加载所有持仓

        info!("State recovery complete");
        Ok(())
    }

    /// 提交写缓存到数据库
    pub fn commit_to_storage(&self) -> Result<(), DexVmError> {
        let mut buffer = self.write_buffer.write();

        // 批量写入账户
        for (addr, account) in buffer.accounts.drain() {
            self.storage.save_account(addr, account)?;
        }

        // 批量写入订单
        for (id, order) in buffer.orders.drain() {
            self.storage.save_order(order)?;
        }

        // 批量写入持仓
        for ((user, pair), position) in buffer.positions.drain() {
            self.storage.save_position(user, pair, position)?;
        }

        // 批量写入成交
        for trade in buffer.trades.drain(..) {
            self.storage.save_trade(trade)?;
        }

        Ok(())
    }

    /// 在执行交易时记录到写缓存
    fn buffer_account_update(&self, address: Address, account: DexAccount) {
        self.write_buffer.write().accounts.insert(address, account);
    }
}
```

**里程碑**:
- ✅ 数据库表定义完成
- ✅ Provider实现完成
- ✅ State集成完成
- ✅ 节点重启可恢复状态

---

### 任务2.2: 签名验证实现 (性能关键)

**时间估算**: 1周

#### 目标
实现真实的ECDSA签名和验证，替换测试签名。

#### 实施步骤

**2.2.1 添加签名依赖**

```toml
# crates/dexvm/primitives/Cargo.toml

[dependencies]
k256 = { version = "0.13", features = ["ecdsa", "std"] }
sha3 = "0.10"
```

**2.2.2 实现签名和验证**

```rust
// crates/dexvm/primitives/src/signature.rs

use k256::ecdsa::{SigningKey, VerifyingKey, signature::Signer, signature::Verifier};
use alloy_primitives::{Address, B256, Signature};
use sha3::Digest;

impl DexTransaction {
    /// 计算交易哈希
    pub fn compute_hash(&self) -> B256 {
        let mut hasher = sha3::Keccak256::new();

        // Hash all transaction fields except signature
        hasher.update(self.sender.as_slice());
        hasher.update(&self.nonce.to_le_bytes());
        hasher.update(&self.gas_limit.to_le_bytes());
        hasher.update(&self.timestamp.to_le_bytes());

        // Hash instruction (RLP encoded)
        let mut buf = Vec::new();
        self.instruction.encode(&mut buf);
        hasher.update(&buf);

        B256::from_slice(&hasher.finalize())
    }
}

impl SignedDexTransaction {
    /// 使用私钥签名交易
    pub fn sign(transaction: DexTransaction, signing_key: &SigningKey) -> Self {
        let hash = transaction.compute_hash();
        let signature: k256::ecdsa::Signature = signing_key.sign(&hash.0);

        // 转换为Alloy格式
        let sig_bytes = signature.to_bytes();
        let r = B256::from_slice(&sig_bytes[..32]);
        let s = B256::from_slice(&sig_bytes[32..]);

        // v = 27 或 28 (EIP-155之前)
        let v = 27u64;

        Self {
            transaction,
            signature: Signature { r, s, v },
        }
    }

    /// 验证签名并恢复发送者地址
    pub fn verify_signature(&self) -> Result<Address, SignatureError> {
        let hash = self.transaction.compute_hash();

        // 转换签名格式
        let mut sig_bytes = [0u8; 64];
        sig_bytes[..32].copy_from_slice(self.signature.r.as_slice());
        sig_bytes[32..].copy_from_slice(self.signature.s.as_slice());

        let signature = k256::ecdsa::Signature::from_bytes(&sig_bytes.into())
            .map_err(|_| SignatureError::InvalidSignature)?;

        // 恢复公钥
        let recovered_key = VerifyingKey::recover_from_prehash(&hash.0, &signature, self.signature.v as u8 - 27)
            .map_err(|_| SignatureError::RecoveryFailed)?;

        // 公钥 -> 地址
        let public_key_bytes = recovered_key.to_encoded_point(false);
        let public_key_hash = sha3::Keccak256::digest(&public_key_bytes.as_bytes()[1..]);
        let address = Address::from_slice(&public_key_hash[12..]);

        // 验证地址匹配
        if address != self.transaction.sender {
            return Err(SignatureError::SenderMismatch);
        }

        Ok(address)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SignatureError {
    #[error("Invalid signature format")]
    InvalidSignature,
    #[error("Signature recovery failed")]
    RecoveryFailed,
    #[error("Sender address mismatch")]
    SenderMismatch,
}
```

**2.2.3 集成到执行器**

```rust
// crates/dexvm/executor/src/block_executor.rs (更新)

impl DexVmBlockExecutor {
    pub fn execute_block(&self, transactions: Vec<SignedDexTransaction>) -> Result<BlockResult> {
        let start = std::time::Instant::now();
        let mut successful = 0;
        let mut failed = 0;
        let mut total_gas = 0u64;

        for signed_tx in transactions {
            // ✅ 真实签名验证
            let sender = match signed_tx.verify_signature() {
                Ok(addr) => addr,
                Err(e) => {
                    warn!("Signature verification failed: {}", e);
                    failed += 1;
                    continue;
                }
            };

            // 执行交易
            match self.state.execute_transaction(&signed_tx) {
                Ok(result) => {
                    successful += 1;
                    total_gas += result.gas_used;
                }
                Err(e) => {
                    warn!("Transaction execution failed: {}", e);
                    failed += 1;
                }
            }
        }

        // ...
    }
}
```

**里程碑**:
- ✅ 签名验证正常工作
- ✅ 无效签名被拒绝
- ✅ 性能影响可接受 (< 100μs per signature)

---

### 任务2.3: 并行执行优化 (性能突破)

**时间估算**: 2-3周

#### 目标
实现交易并行执行，利用多核CPU提升TPS。

#### 实施策略

**2.3.1 交易依赖分析**

```rust
// crates/dexvm/executor/src/parallel.rs

use rayon::prelude::*;

/// 分析交易之间的依赖关系
pub struct DependencyAnalyzer;

impl DependencyAnalyzer {
    /// 检测两个交易是否有冲突
    pub fn has_conflict(tx1: &SignedDexTransaction, tx2: &SignedDexTransaction) -> bool {
        // 同一发送者 -> 有依赖（nonce顺序）
        if tx1.transaction.sender == tx2.transaction.sender {
            return true;
        }

        // 操作同一交易对 -> 可能冲突
        match (&tx1.transaction.instruction, &tx2.transaction.instruction) {
            (
                DexInstruction::PlaceLimitOrder { pair: p1, .. },
                DexInstruction::PlaceLimitOrder { pair: p2, .. },
            ) => p1 == p2,

            (
                DexInstruction::CancelOrder { order_id: id1 },
                DexInstruction::CancelOrder { order_id: id2 },
            ) => id1 == id2,

            // Deposit/Withdraw是账户操作，已经通过sender检查
            _ => false,
        }
    }

    /// 将交易分组为独立批次
    pub fn partition_transactions(
        transactions: Vec<SignedDexTransaction>
    ) -> Vec<Vec<SignedDexTransaction>> {
        let mut batches = Vec::new();
        let mut current_batch = Vec::new();
        let mut touched_senders = HashSet::new();
        let mut touched_pairs = HashSet::new();

        for tx in transactions {
            let sender = tx.transaction.sender;

            // 提取交易对（如果有）
            let pair = match &tx.transaction.instruction {
                DexInstruction::PlaceLimitOrder { pair, .. } => Some(pair.clone()),
                _ => None,
            };

            // 检查冲突
            let has_conflict = touched_senders.contains(&sender) ||
                (pair.is_some() && touched_pairs.contains(&pair.unwrap()));

            if has_conflict {
                // 开始新批次
                batches.push(std::mem::take(&mut current_batch));
                touched_senders.clear();
                touched_pairs.clear();
            }

            // 添加到当前批次
            current_batch.push(tx);
            touched_senders.insert(sender);
            if let Some(p) = pair {
                touched_pairs.insert(p);
            }
        }

        if !current_batch.is_empty() {
            batches.push(current_batch);
        }

        batches
    }
}
```

**2.3.2 并行执行器**

```rust
// crates/dexvm/executor/src/parallel.rs

pub struct ParallelDexVmExecutor<DB: Database> {
    state: Arc<DexVmState<DB>>,
    thread_pool: rayon::ThreadPool,
}

impl<DB: Database> ParallelDexVmExecutor<DB> {
    pub fn new(state: Arc<DexVmState<DB>>, num_threads: usize) -> Self {
        let thread_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build()
            .unwrap();

        Self { state, thread_pool }
    }

    /// 并行执行区块
    pub fn execute_block_parallel(
        &self,
        transactions: Vec<SignedDexTransaction>,
    ) -> Result<BlockResult> {
        let start = Instant::now();

        // 1. 分析依赖，划分批次
        let batches = DependencyAnalyzer::partition_transactions(transactions);

        let mut total_successful = 0;
        let mut total_failed = 0;
        let mut total_gas = 0u64;

        // 2. 批次顺序执行，批次内并行执行
        for batch in batches {
            // 并行验证签名
            let verified: Vec<_> = self.thread_pool.install(|| {
                batch.par_iter()
                    .map(|tx| {
                        tx.verify_signature()
                            .map(|sender| (tx.clone(), sender))
                    })
                    .collect()
            });

            // 并行执行交易
            let results: Vec<_> = self.thread_pool.install(|| {
                verified.par_iter()
                    .filter_map(|v| v.as_ref().ok())
                    .map(|(tx, sender)| {
                        self.state.execute_transaction(tx)
                    })
                    .collect()
            });

            // 统计结果
            for result in results {
                match result {
                    Ok(exec_result) => {
                        total_successful += 1;
                        total_gas += exec_result.gas_used;
                    }
                    Err(_) => {
                        total_failed += 1;
                    }
                }
            }
        }

        let execution_time_ms = start.elapsed().as_millis() as u64;
        let total_txs = total_successful + total_failed;
        let tps = if execution_time_ms > 0 {
            (total_txs as f64 / execution_time_ms as f64) * 1000.0
        } else {
            0.0
        };

        Ok(BlockResult {
            block_number: 0,
            successful_txs: total_successful,
            failed_txs: total_failed,
            total_gas,
            execution_time_ms,
            tps,
        })
    }
}
```

**优化效果预估**:
- 签名验证并行化: **4-8x** 提升
- 交易执行并行化: **2-4x** 提升（受订单簿锁限制）
- 综合提升: **10-30x** -> 从3.4K TPS到 **34K-100K TPS**

**里程碑**:
- ✅ 并行签名验证工作正常
- ✅ 无数据竞争
- ✅ TPS显著提升

---

### 任务2.4: EVM接口预留 (架构准备)

**时间估算**: 1周

#### 目标
定义EVM接口但不实现，保持与DexVM隔离。

#### 实施步骤

**2.4.1 定义EVM执行器接口**

```rust
// crates/evm/src/dex_evm_placeholder.rs

use alloy_primitives::{Address, B256, U256};

/// EVM执行器（预留接口）
pub struct EvmBlockExecutor {
    _phantom: std::marker::PhantomData<()>,
}

impl EvmBlockExecutor {
    pub fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }

    /// 执行EVM交易（未启用）
    pub fn execute_transaction(
        &self,
        _tx: &EvmTransaction,
    ) -> Result<EvmExecutionResult, EvmError> {
        Err(EvmError::NotEnabled)
    }

    /// 调用合约（未启用）
    pub fn call(
        &self,
        _from: Address,
        _to: Address,
        _data: Vec<u8>,
    ) -> Result<Vec<u8>, EvmError> {
        Err(EvmError::NotEnabled)
    }
}

#[derive(Debug, Clone)]
pub struct EvmTransaction {
    pub from: Address,
    pub to: Option<Address>,
    pub value: U256,
    pub data: Vec<u8>,
    pub nonce: u64,
    pub gas_limit: u64,
}

pub struct EvmExecutionResult {
    pub gas_used: u64,
    pub output: Vec<u8>,
    pub logs: Vec<EvmLog>,
}

pub struct EvmLog {
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum EvmError {
    #[error("EVM is not enabled in this build")]
    NotEnabled,
    #[error("EVM execution error: {0}")]
    ExecutionError(String),
}
```

**2.4.2 统一交易枚举**

```rust
// crates/primitives/src/transaction.rs

/// 统一交易类型
#[derive(Debug, Clone)]
pub enum HybridTransaction {
    /// DexVM交易（当前使用）
    Dex(SignedDexTransaction),

    /// EVM交易（预留）
    Evm(EvmTransaction),
}

impl HybridTransaction {
    pub fn sender(&self) -> Address {
        match self {
            HybridTransaction::Dex(tx) => tx.transaction.sender,
            HybridTransaction::Evm(tx) => tx.from,
        }
    }

    pub fn nonce(&self) -> u64 {
        match self {
            HybridTransaction::Dex(tx) => tx.transaction.nonce,
            HybridTransaction::Evm(tx) => tx.nonce,
        }
    }
}
```

**2.4.3 在区块执行器中路由**

```rust
// crates/dexvm/executor/src/hybrid_executor.rs

pub struct HybridBlockExecutor<DB: Database> {
    dex_executor: ParallelDexVmExecutor<DB>,
    evm_executor: EvmBlockExecutor,  // 预留
}

impl<DB: Database> HybridBlockExecutor<DB> {
    pub fn execute_block(
        &self,
        transactions: Vec<HybridTransaction>,
    ) -> Result<BlockResult> {
        // 分类交易
        let mut dex_txs = Vec::new();
        let mut evm_txs = Vec::new();

        for tx in transactions {
            match tx {
                HybridTransaction::Dex(dtx) => dex_txs.push(dtx),
                HybridTransaction::Evm(etx) => evm_txs.push(etx),
            }
        }

        // 执行DexVM交易（主要路径）
        let dex_result = self.dex_executor.execute_block_parallel(dex_txs)?;

        // EVM交易返回错误（未启用）
        for _etx in evm_txs {
            warn!("EVM transaction received but EVM is not enabled");
            // 静默跳过或返回错误
        }

        Ok(dex_result)
    }
}
```

**里程碑**:
- ✅ EVM接口定义完整
- ✅ 不影响DexVM性能
- ✅ 未来易于启用

---

### 任务2.5: 完善功能 (用户体验)

**时间估算**: 1周

#### 2.5.1 完成取消订单功能

```rust
// crates/dexvm/core/src/matching_engine.rs (更新)

pub struct MatchingEngine {
    orderbooks: DashMap<TradingPair, RwLock<OrderBook>>,

    /// 新增：用户订单索引
    user_orders: DashMap<Address, HashSet<OrderId>>,

    /// 新增：订单 -> 交易对映射
    order_to_pair: DashMap<OrderId, TradingPair>,
}

impl MatchingEngine {
    pub fn place_order(&self, order: Order) -> Result<OrderId, MatchingError> {
        let pair = order.pair;
        let order_id = order.id;
        let user = order.maker;

        // 1. 添加到订单簿
        let book = self.orderbooks
            .entry(pair)
            .or_insert_with(|| RwLock::new(OrderBook::new(pair)));

        book.write().add_order(order)?;

        // 2. 更新用户订单索引
        self.user_orders
            .entry(user)
            .or_insert_with(HashSet::new)
            .insert(order_id);

        // 3. 记录订单-交易对映射
        self.order_to_pair.insert(order_id, pair);

        Ok(order_id)
    }

    /// ✅ 完整实现取消订单
    pub fn cancel_order(&self, user: Address, order_id: OrderId) -> Result<Order, MatchingError> {
        // 1. 检查订单是否属于该用户
        let user_orders = self.user_orders.get(&user)
            .ok_or(MatchingError::UserHasNoOrders)?;

        if !user_orders.contains(&order_id) {
            return Err(MatchingError::OrderNotFound);
        }

        // 2. 查找订单所属交易对
        let pair = self.order_to_pair.get(&order_id)
            .ok_or(MatchingError::OrderNotFound)?
            .clone();

        // 3. 从订单簿中移除
        let book = self.orderbooks.get(&pair)
            .ok_or(MatchingError::OrderBookNotFound)?;

        let canceled_order = book.write().cancel_order(&order_id)?;

        // 4. 清理索引
        if let Some(mut user_orders) = self.user_orders.get_mut(&user) {
            user_orders.remove(&order_id);
        }
        self.order_to_pair.remove(&order_id);

        Ok(canceled_order)
    }

    /// 获取用户所有活跃订单
    pub fn get_user_orders(&self, user: &Address) -> Vec<OrderId> {
        self.user_orders
            .get(user)
            .map(|orders| orders.iter().cloned().collect())
            .unwrap_or_default()
    }
}
```

#### 2.5.2 改进查询API

```rust
// crates/dexvm/core/src/state.rs (更新)

use serde_json;

impl<DB: Database> DexVmState<DB> {
    /// 查询订单簿（格式化输出）
    pub fn query_orderbook(
        &self,
        pair: TradingPair,
        depth: usize,
    ) -> Result<Vec<u8>, DexVmError> {
        let (bids, asks) = self.matching_engine.get_orderbook_depth(pair, depth)?;

        #[derive(Serialize)]
        struct OrderBookResponse {
            pair: String,
            timestamp: u64,
            bids: Vec<PriceLevel>,
            asks: Vec<PriceLevel>,
        }

        #[derive(Serialize)]
        struct PriceLevel {
            price: String,
            amount: String,
        }

        let response = OrderBookResponse {
            pair: format!("{:?}", pair),
            timestamp: *self.current_timestamp.read(),
            bids: bids.iter()
                .map(|(p, a)| PriceLevel {
                    price: p.to_string(),
                    amount: a.to_string(),
                })
                .collect(),
            asks: asks.iter()
                .map(|(p, a)| PriceLevel {
                    price: p.to_string(),
                    amount: a.to_string(),
                })
                .collect(),
        };

        Ok(serde_json::to_vec(&response).unwrap())
    }

    /// 查询用户订单
    pub fn query_user_orders(&self, user: Address) -> Result<Vec<u8>, DexVmError> {
        let order_ids = self.matching_engine.get_user_orders(&user);

        // 从存储加载完整订单信息
        let mut orders = Vec::new();
        for order_id in order_ids {
            if let Some(order) = self.storage.get_order(&order_id)? {
                orders.push(order);
            }
        }

        Ok(serde_json::to_vec(&orders).unwrap())
    }

    /// 查询账户信息
    pub fn query_account(&self, user: Address) -> Result<Vec<u8>, DexVmError> {
        let state = self.accounts.read();
        let account = state.get(&user).ok_or(DexVmError::AccountNotFound)?;

        Ok(serde_json::to_vec(&account).unwrap())
    }
}
```

**里程碑**:
- ✅ 取消订单正常工作
- ✅ 查询API返回JSON格式
- ✅ 用户体验改善

---

## 🎯 第二阶段总结

### 核心交付物

1. **持久化存储**
   - ✅ MDBX集成
   - ✅ 状态恢复
   - ✅ 数据库表结构

2. **性能优化**
   - ✅ 真实签名验证
   - ✅ 并行执行
   - ✅ 预期TPS: 50K-200K

3. **架构准备**
   - ✅ EVM接口预留
   - ✅ 双VM状态隔离
   - ✅ 统一交易路由

4. **功能完善**
   - ✅ 取消订单
   - ✅ 改进查询
   - ✅ JSON响应

### 性能预期

| 优化项 | 提升倍数 | 估算TPS |
|--------|---------|---------|
| 基线（第一阶段） | 1x | 3,400 |
| + 并行签名验证 | 4-8x | 13K-27K |
| + 并行交易执行 | 2-4x | 26K-108K |
| + 其他优化 | 1.5-2x | **40K-200K** ✅ |

---

## 📋 第三阶段预览：极致优化与生产就绪

### 3.1 订单簿引擎优化

**目标**: 突破200K TPS瓶颈

1. **无锁订单簿**
   - 使用lock-free数据结构
   - 基于atomic操作
   - 减少锁竞争

2. **SIMD加速**
   - 批量价格比较
   - 向量化撮合算法
   - 2-4x性能提升

3. **零拷贝优化**
   - 使用`rkyv`序列化
   - 内存映射订单簿
   - 减少allocations

4. **JIT编译（可选）**
   - 热路径编译为机器码
   - 类似LuaJIT策略

### 3.2 网络与RPC

1. **DexVM专用RPC**
   - `dex_submitOrder`
   - `dex_cancelOrder`
   - `dex_getOrderBook`
   - `dex_getUserOrders`

2. **WebSocket订阅**
   - 实时订单簿更新
   - 成交推送
   - 持仓变化通知

3. **批量接口**
   - `dex_submitOrders` (批量下单)
   - 减少网络往返

### 3.3 监控与可观测性

1. **Metrics**
   - TPS实时统计
   - 订单簿深度
   - 延迟百分位

2. **日志**
   - 结构化日志
   - 链路追踪
   - 错误告警

3. **性能分析**
   - Flamegraph支持
   - CPU profiling
   - Memory profiling

### 3.4 安全加固

1. **DoS防护**
   - 订单频率限制
   - Gas机制完善
   - 恶意订单检测

2. **状态一致性**
   - Merkle state root
   - 状态快照与回滚
   - 数据完整性校验

3. **审计日志**
   - 所有状态变更记录
   - 可审计性
   - 合规支持

---

## 📊 优化阶段总览

| 阶段 | 目标TPS | 关键任务 | 时间估算 |
|------|---------|----------|----------|
| **第一阶段 (已完成)** | MVP | 订单簿+基础执行 | ✅ 完成 |
| **第二阶段 (当前)** | 50K-200K | 持久化+并行化+签名 | 6-8周 |
| **第三阶段** | 200K+ | 极致优化+监控 | 4-6周 |
| **生产就绪** | 稳定运行 | 安全+运维+审计 | 2-3周 |

---

## 🚀 下一步行动

### 立即开始 (本周)

1. **创建storage crate**
   ```bash
   cargo new --lib crates/dexvm/storage
   ```

2. **定义数据库表**
   - 实现`tables.rs`
   - 定义所有表结构

3. **实现StorageProvider**
   - CRUD操作
   - 批量写入
   - 事务管理

### 一周内完成

1. **集成MDBX到State**
2. **实现状态恢复逻辑**
3. **测试持久化功能**

### 两周内完成

1. **实现签名验证**
2. **集成k256库**
3. **性能测试签名验证开销**

### 一个月内完成

1. **实现并行执行**
2. **依赖分析算法**
3. **并行benchmark测试**

---

## 📝 关键决策记录

### 为什么EVM只预留不启用?

1. **性能优先**
   - DexVM专注优化可达200K TPS
   - EVM会引入复杂性和开销
   - 订单撮合不需要图灵完备

2. **架构简洁**
   - 单一执行路径
   - 更容易优化和调试
   - 减少状态同步复杂度

3. **渐进式演进**
   - 先验证DexVM可行性
   - 市场需求再启用EVM
   - 保持未来扩展性

### 为什么此时引入持久化?

1. **节点可用性**
   - 重启不丢失状态
   - 支持长期运行
   - 灾难恢复能力

2. **历史数据**
   - 成交记录查询
   - 审计追溯
   - 数据分析

3. **性能优化基础**
   - 冷热数据分离
   - 内存压力释放
   - 批量写入优化

### 为什么选择并行执行?

1. **性能瓶颈**
   - 第一阶段3.4K TPS远低于目标
   - 订单簿本身是纳秒级
   - CPU未充分利用

2. **可行性**
   - 交易天然独立（不同pair）
   - Rayon库成熟
   - 风险可控

3. **效益**
   - 10-30x性能提升
   - 接近或达到200K目标
   - 成本效益高

---

## 🎓 技术风险与缓解

### 风险1: 并行化导致状态不一致

**缓解**:
- 依赖分析算法验证
- 增加并发测试
- 使用原子操作保护关键路径

### 风险2: MDBX性能不及预期

**缓解**:
- 使用写缓存批量提交
- 异步持久化（写完内存即返回）
- 可切换到rocksdb

### 风险3: 签名验证成为瓶颈

**缓解**:
- 并行验证
- 使用SIMD优化库
- 考虑签名批量验证算法

---

## 📚 参考资料

### Reth相关
- `crates/storage/db` - 数据库抽象
- `crates/evm` - EVM实现参考
- `crates/node` - 节点构建器

### 性能优化
- [Rayon并行编程](https://github.com/rayon-rs/rayon)
- [rkyv零拷贝](https://github.com/rkyv/rkyv)
- [无锁数据结构](https://preshing.com/20120612/an-introduction-to-lock-free-programming/)

### 签名验证
- [k256 ECDSA](https://docs.rs/k256/)
- [EIP-155 签名](https://eips.ethereum.org/EIPS/eip-155)

---

**文档版本**: v2.0
**最后更新**: 2026-01-14
**作者**: Claude Code
**状态**: 第二阶段规划完成
