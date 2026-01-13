# 双VM架构：区块结构与状态存储设计

## 1. 区块数据结构设计

### 1.1 设计目标

```
需求：
1. 一个区块同时包含EVM和DEXVM交易
2. 支持并行验证（EVM和DEXVM交易可并行）
3. 兼容Reth现有架构
4. 状态根能够统一验证
5. 支持独立的交易费用市场

挑战：
1. EVM和DEXVM交易格式完全不同
2. Gas机制不同（EVM gas vs DEXVM固定费用）
3. 执行结果格式不同
4. 需要保持向后兼容性
```

### 1.2 方案对比

#### 方案A：单一交易列表 + 类型标记

```rust
/// 混合区块结构
pub struct HybridBlock {
    /// 区块头
    pub header: BlockHeader,

    /// 混合交易列表（EVM + DEXVM）
    pub transactions: Vec<Transaction>,

    /// Ommers（叔块，如果需要）
    pub ommers: Vec<Header>,
}

/// 交易枚举：区分EVM和DEXVM
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transaction {
    /// 标准EVM交易
    Evm(EvmTransaction),

    /// DEXVM交易（订单操作）
    Dex(DexTransaction),
}

/// 标准EVM交易
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmTransaction {
    pub nonce: u64,
    pub gas_price: u128,
    pub gas_limit: u64,
    pub to: Option<Address>,
    pub value: U256,
    pub data: Bytes,
    pub signature: Signature,
}

/// DEXVM交易（极简设计）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexTransaction {
    /// 用户地址
    pub from: Address,

    /// 交易类型
    pub tx_type: DexTxType,

    /// DEXVM字节码（或结构化数据）
    pub payload: DexPayload,

    /// 签名
    pub signature: Signature,

    /// Nonce（防重放）
    pub nonce: u64,
}

/// DEXVM交易类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DexTxType {
    /// Spot下单
    SpotOrder = 0x00,

    /// Spot撤单
    SpotCancel = 0x01,

    /// Perp开仓
    PerpOpen = 0x10,

    /// Perp平仓
    PerpClose = 0x11,

    /// Perp调整保证金
    PerpAdjustMargin = 0x12,

    /// Spot ↔ Perp 转账
    Transfer = 0x20,

    /// 批量操作
    Batch = 0xFF,
}

/// DEXVM交易载荷
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DexPayload {
    /// Spot订单
    SpotOrder(SpotOrderPayload),

    /// Spot撤单
    SpotCancel { order_id: B256 },

    /// Perp开仓
    PerpOpen(PerpOrderPayload),

    /// Perp平仓
    PerpClose(PerpClosePayload),

    /// 保证金调整
    AdjustMargin { symbol: B256, amount: i128 },

    /// Spot ↔ Perp 转账
    Transfer(TransferPayload),

    /// 批量操作（多个操作打包）
    Batch(Vec<DexPayload>),
}

/// Spot订单载荷（紧凑编码）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotOrderPayload {
    /// 交易对（32字节）
    pub symbol: B256,

    /// 买/卖方向
    pub side: Side,

    /// 订单类型
    pub order_type: OrderType,

    /// 价格（定点数，8位小数）
    pub price: u64,

    /// 数量（定点数，8位小数）
    pub quantity: u64,
}

/// Perp订单载荷
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpOrderPayload {
    /// 合约标识
    pub symbol: B256,

    /// 多/空方向
    pub side: Side,

    /// 订单类型
    pub order_type: OrderType,

    /// 价格
    pub price: u64,

    /// 数量
    pub quantity: u64,

    /// 杠杆倍数
    pub leverage: u8,

    /// 只减仓（true = 只平仓）
    pub reduce_only: bool,
}

// 优势：
// 1. 简单直观，一个交易列表
// 2. 容易理解和实现
// 3. 交易顺序明确
//
// 劣势：
// 1. 序列化时需要区分类型
// 2. 难以并行处理（需要先分类）
// 3. 与Reth现有结构不太兼容
```

#### 方案B：分离的交易列表（推荐）⭐

```rust
/// 混合区块结构（推荐）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HybridBlock {
    /// 区块头
    pub header: HybridBlockHeader,

    /// EVM交易列表
    pub evm_body: EvmBlockBody,

    /// DEXVM交易列表
    pub dex_body: DexBlockBody,
}

/// 混合区块头
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HybridBlockHeader {
    /// 标准以太坊区块头字段
    pub parent_hash: B256,
    pub ommers_hash: B256,
    pub beneficiary: Address,
    pub state_root: B256,          // 统一的状态根（EVM + DEXVM）
    pub timestamp: u64,
    pub number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub mix_hash: B256,
    pub nonce: u64,
    pub base_fee_per_gas: Option<u64>,
    pub blob_gas_used: Option<u64>,
    pub excess_blob_gas: Option<u64>,
    pub parent_beacon_block_root: Option<B256>,

    // === 新增：DEXVM相关字段 ===

    /// EVM交易根
    pub evm_transactions_root: B256,

    /// DEXVM交易根
    pub dex_transactions_root: B256,

    /// EVM收据根
    pub evm_receipts_root: B256,

    /// DEXVM收据根
    pub dex_receipts_root: B256,

    /// DEXVM交易数量
    pub dex_tx_count: u32,

    /// DEXVM总手续费
    pub dex_fees_collected: u128,
}

/// EVM区块体
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmBlockBody {
    /// EVM交易（标准格式）
    pub transactions: Vec<EvmTransaction>,

    /// Ommers（如果需要）
    pub ommers: Vec<Header>,
}

/// DEXVM区块体
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexBlockBody {
    /// DEXVM交易列表
    pub transactions: Vec<DexTransaction>,
}

// 优势：
// 1. 清晰分离，便于并行处理
// 2. 各自独立的Merkle根，便于验证
// 3. EVM部分完全兼容以太坊
// 4. 便于独立的gas/fee市场
//
// 劣势：
// 1. 结构稍复杂
// 2. 需要定义跨VM交易的顺序语义
```

#### 方案C：Reth兼容扩展（最推荐）⭐⭐

```rust
use reth_primitives::{Block as RethBlock, BlockBody, Header, TransactionSigned};

/// 扩展Reth的区块结构
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexBlock {
    /// 标准Reth区块（EVM部分）
    pub reth_block: RethBlock,

    /// DEXVM扩展
    pub dex_extension: DexBlockExtension,
}

/// DEXVM扩展（不影响原有结构）
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DexBlockExtension {
    /// DEXVM交易
    pub transactions: Vec<DexTransaction>,

    /// DEXVM执行结果
    pub receipts: Vec<DexReceipt>,

    /// DEXVM状态变更
    pub state_changes: Vec<DexStateChange>,
}

impl DexBlock {
    /// 获取所有交易（EVM + DEXVM）
    pub fn all_transactions(&self) -> impl Iterator<Item = AnyTransaction> + '_ {
        self.reth_block
            .body
            .transactions
            .iter()
            .map(|tx| AnyTransaction::Evm(tx.clone()))
            .chain(
                self.dex_extension
                    .transactions
                    .iter()
                    .map(|tx| AnyTransaction::Dex(tx.clone()))
            )
    }

    /// 计算统一状态根
    pub fn compute_state_root(&self) -> B256 {
        // 合并EVM和DEXVM的状态根
        let evm_root = self.reth_block.header.state_root;
        let dex_root = self.compute_dex_state_root();

        // 组合方式1：简单哈希
        keccak256(&[evm_root.as_slice(), dex_root.as_slice()].concat())

        // 组合方式2：统一MPT（更复杂但兼容性更好）
        // self.compute_unified_mpt_root()
    }
}

// 优势：
// 1. 完全兼容Reth现有架构
// 2. EVM部分零改动
// 3. DEXVM作为扩展，可选启用
// 4. 升级路径清晰
//
// 劣势：
// 1. 需要处理状态根的合并逻辑
```

### 1.3 最终推荐：方案C + 优化

```rust
/// 最终区块设计
pub mod block {
    use alloy_primitives::{Address, Bytes, B256, U256};
    use reth_primitives::{Block as RethBlock, Header, TransactionSigned};

    /// DEX区块（Reth兼容 + DEXVM扩展）
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct DexBlock {
        /// 区块头（扩展）
        pub header: DexBlockHeader,

        /// EVM交易体
        pub evm_body: EvmBlockBody,

        /// DEXVM交易体
        pub dex_body: DexBlockBody,
    }

    /// 扩展的区块头
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct DexBlockHeader {
        /// 标准以太坊头（完全兼容）
        pub eth_header: Header,

        /// DEXVM扩展哈希（存在extra_data或新字段）
        pub dex_data_hash: B256,
    }

    /// EVM区块体（标准）
    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct EvmBlockBody {
        pub transactions: Vec<TransactionSigned>,
        pub ommers: Vec<Header>,
        pub withdrawals: Option<Vec<Withdrawal>>,
    }

    /// DEXVM区块体
    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct DexBlockBody {
        /// Spot交易
        pub spot_txs: Vec<SpotTransaction>,

        /// Perp交易
        pub perp_txs: Vec<PerpTransaction>,

        /// 批量操作（打包多个订单）
        pub batch_txs: Vec<BatchTransaction>,
    }

    /// Spot交易（紧凑编码）
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[repr(C)]
    pub struct SpotTransaction {
        /// 用户地址（20字节）
        pub from: Address,

        /// 交易对ID（4字节，预定义映射）
        pub symbol_id: u32,

        /// 操作类型 + 方向（1字节）
        /// bits 0-3: 操作类型（0=下单, 1=撤单, 2=改单）
        /// bits 4-7: 方向（0=买, 1=卖）
        pub op_flags: u8,

        /// 价格（8字节，定点数）
        pub price: u64,

        /// 数量（8字节，定点数）
        pub quantity: u64,

        /// 订单ID（32字节，撤单/改单时使用）
        pub order_id: Option<B256>,

        /// 签名（65字节）
        pub signature: CompactSignature,

        /// Nonce（8字节）
        pub nonce: u64,
    }
    // 总大小：~120字节（vs EVM交易 ~200字节）

    /// Perp交易
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[repr(C)]
    pub struct PerpTransaction {
        pub from: Address,
        pub symbol_id: u32,
        pub op_flags: u8,        // 包含多/空方向、只减仓标志
        pub price: u64,
        pub quantity: u64,
        pub leverage: u8,        // 1-125x
        pub order_id: Option<B256>,
        pub signature: CompactSignature,
        pub nonce: u64,
    }
    // 总大小：~125字节

    /// 批量交易（节省签名开销）
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BatchTransaction {
        pub from: Address,
        pub operations: Vec<Operation>,  // 多个操作
        pub signature: CompactSignature, // 一个签名
        pub nonce: u64,
    }

    /// 单个操作
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Operation {
        SpotOrder { symbol_id: u32, side: Side, price: u64, quantity: u64 },
        SpotCancel { order_id: B256 },
        PerpOrder { symbol_id: u32, side: Side, price: u64, quantity: u64, leverage: u8 },
        PerpClose { symbol_id: u32, quantity: u64 },
    }

    /// 紧凑签名（65字节）
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CompactSignature {
        pub r: B256,
        pub s: B256,
        pub v: u8,
    }
}
```

### 1.4 区块序列化与编码

```rust
/// 区块RLP编码
impl Encodable for DexBlock {
    fn encode(&self, out: &mut dyn BufMut) {
        // 版本标识（1字节）
        out.put_u8(0x01); // Version 1

        // EVM部分（标准RLP）
        Header::encode(&self.header.eth_header, out);
        self.evm_body.encode(out);

        // DEXVM部分（自定义编码）
        self.encode_dex_body(out);
    }

    fn length(&self) -> usize {
        1 + // version
        self.header.eth_header.length() +
        self.evm_body.length() +
        self.dex_body.length()
    }
}

impl DexBlock {
    /// DEXVM部分编码（优化的紧凑格式）
    fn encode_dex_body(&self, out: &mut dyn BufMut) {
        // Spot交易数量（4字节）
        out.put_u32(self.dex_body.spot_txs.len() as u32);

        // Spot交易（紧凑编码）
        for tx in &self.dex_body.spot_txs {
            tx.encode_compact(out);
        }

        // Perp交易数量（4字节）
        out.put_u32(self.dex_body.perp_txs.len() as u32);

        // Perp交易
        for tx in &self.dex_body.perp_txs {
            tx.encode_compact(out);
        }

        // 批量交易数量
        out.put_u32(self.dex_body.batch_txs.len() as u32);

        // 批量交易
        for tx in &self.dex_body.batch_txs {
            tx.encode(out);
        }
    }
}

/// Spot交易紧凑编码
impl SpotTransaction {
    pub fn encode_compact(&self, out: &mut dyn BufMut) {
        out.put_slice(self.from.as_slice());     // 20 bytes
        out.put_u32(self.symbol_id);             // 4 bytes
        out.put_u8(self.op_flags);               // 1 byte
        out.put_u64(self.price);                 // 8 bytes
        out.put_u64(self.quantity);              // 8 bytes

        if let Some(order_id) = &self.order_id {
            out.put_u8(1);                       // has order_id
            out.put_slice(order_id.as_slice());  // 32 bytes
        } else {
            out.put_u8(0);                       // no order_id
        }

        out.put_slice(&self.signature.r.0);     // 32 bytes
        out.put_slice(&self.signature.s.0);     // 32 bytes
        out.put_u8(self.signature.v);           // 1 byte
        out.put_u64(self.nonce);                // 8 bytes
    }

    // 总计：120字节（固定大小，无需长度前缀）
}
```

### 1.5 区块大小与容量分析

```rust
/// 区块容量计算
pub struct BlockCapacity {
    /// 区块大小限制（字节）
    pub max_block_size: usize,

    /// EVM gas限制
    pub evm_gas_limit: u64,
}

impl BlockCapacity {
    /// 默认配置
    pub fn default() -> Self {
        Self {
            max_block_size: 10 * 1024 * 1024,  // 10 MB
            evm_gas_limit: 30_000_000,          // 30M gas
        }
    }

    /// 计算容量
    pub fn estimate_capacity(&self) -> CapacityEstimate {
        // EVM交易平均大小：~200字节
        // EVM交易平均gas：~50,000
        let max_evm_txs_by_gas = self.evm_gas_limit / 50_000;  // ~600笔
        let evm_size = max_evm_txs_by_gas as usize * 200;       // ~120 KB

        // DEXVM交易平均大小：~120字节
        let remaining_size = self.max_block_size - evm_size;
        let max_dex_txs = remaining_size / 120;                  // ~82,000笔

        CapacityEstimate {
            max_evm_txs: max_evm_txs_by_gas as usize,  // ~600笔
            max_dex_txs,                                 // ~82,000笔
            total_tps: self.calculate_tps(max_evm_txs_by_gas as usize, max_dex_txs),
        }
    }

    /// 计算TPS（假设500ms出块）
    fn calculate_tps(&self, evm_txs: usize, dex_txs: usize) -> usize {
        (evm_txs + dex_txs) * 2  // 2块/秒
    }
}

/// 容量估算结果
pub struct CapacityEstimate {
    pub max_evm_txs: usize,   // 每块最多EVM交易数
    pub max_dex_txs: usize,   // 每块最多DEXVM交易数
    pub total_tps: usize,     // 总TPS
}

// 示例计算：
// 区块大小：10 MB
// 出块时间：500 ms
// EVM交易：600笔/块
// DEXVM交易：82,000笔/块
// 总TPS：(600 + 82,000) * 2 = 165,200 TPS ✅
```

## 2. DEXVM状态存储设计

### 2.1 状态分层架构

```
┌─────────────────────────────────────────────────────────┐
│                    Memory Layer (L0)                     │
│                   极热数据，微秒级访问                     │
├─────────────────────────────────────────────────────────┤
│  - 活跃订单簿（内存中的BTreeMap）                         │
│  - 持仓索引（快速查找）                                   │
│  - 价格缓存（最新成交价）                                 │
│  - 撮合引擎状态                                          │
└─────────────────────────────────────────────────────────┘
                          ↓ 写入触发
┌─────────────────────────────────────────────────────────┐
│                    MDBX Hot Tables (L1)                  │
│                   热数据，毫秒级访问                       │
├─────────────────────────────────────────────────────────┤
│  DEXVM专用表：                                           │
│  ├─ SpotBalances: 用户现货余额                           │
│  ├─ PerpPositions: 永续合约持仓                          │
│  ├─ PerpMargins: 保证金账户                              │
│  ├─ ActiveOrders: 未成交订单索引                         │
│  ├─ OrderBookSnapshots: 订单簿快照（定期）               │
│  └─ LiquidationQueue: 待清算持仓队列                      │
└─────────────────────────────────────────────────────────┘
                          ↓ 区块确认后写入
┌─────────────────────────────────────────────────────────┐
│                 MDBX Archive Tables (L2)                 │
│                    归档数据，秒级访问                      │
├─────────────────────────────────────────────────────────┤
│  ├─ TradeHistory: 成交历史                               │
│  ├─ OrderHistory: 订单历史                               │
│  ├─ LiquidationHistory: 强平记录                         │
│  ├─ FundingRateHistory: 资金费率历史                     │
│  └─ BalanceSnapshots: 余额快照（每小时）                 │
└─────────────────────────────────────────────────────────┘
                          ↓ 定期归档（>24小时）
┌─────────────────────────────────────────────────────────┐
│              Static Files (L3) - Cold Storage            │
│                    冷数据，分钟级访问                      │
├─────────────────────────────────────────────────────────┤
│  ├─ Trades_{date}.seg: 历史成交                          │
│  ├─ Orders_{date}.seg: 历史订单                          │
│  ├─ Liquidations_{date}.seg: 历史强平                    │
│  └─ Candles_{symbol}_{interval}.seg: K线数据             │
└─────────────────────────────────────────────────────────┘
```

### 2.2 MDBX表结构定义

```rust
use reth_db::{table, DatabaseError};
use reth_primitives::Address;

/// Spot余额表
#[derive(Debug)]
pub struct SpotBalances;

impl table::Table for SpotBalances {
    /// Key: (Address, AssetId)
    type Key = (Address, u32);

    /// Value: 余额信息
    type Value = SpotBalance;

    const NAME: &'static str = "SpotBalances";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotBalance {
    /// 可用余额
    pub available: u128,

    /// 冻结余额（挂单占用）
    pub frozen: u128,

    /// 最后更新区块
    pub last_updated: u64,
}

/// Perp持仓表
#[derive(Debug)]
pub struct PerpPositions;

impl table::Table for PerpPositions {
    /// Key: (Address, SymbolId)
    type Key = (Address, u32);

    /// Value: 持仓信息
    type Value = PerpPosition;

    const NAME: &'static str = "PerpPositions";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpPosition {
    /// 持仓数量（正=多，负=空）
    pub size: i128,

    /// 开仓均价
    pub entry_price: u64,

    /// 占用保证金
    pub margin: u128,

    /// 杠杆倍数
    pub leverage: u8,

    /// 未实现盈亏（缓存值）
    pub unrealized_pnl: i128,

    /// 强平价格
    pub liquidation_price: u64,

    /// 最后更新区块
    pub last_updated: u64,

    /// 最后资金费率结算时间
    pub last_funding_time: u64,
}

/// Perp保证金表
#[derive(Debug)]
pub struct PerpMargins;

impl table::Table for PerpMargins {
    /// Key: (Address, AssetId)
    type Key = (Address, u32);

    /// Value: 保证金信息
    type Value = PerpMargin;

    const NAME: &'static str = "PerpMargins";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpMargin {
    /// 总保证金
    pub total: u128,

    /// 已占用（所有持仓）
    pub used: u128,

    /// 可用保证金
    pub available: u128,

    /// 累计盈亏
    pub realized_pnl: i128,

    /// 最后更新区块
    pub last_updated: u64,
}

/// 活跃订单索引表
#[derive(Debug)]
pub struct ActiveOrders;

impl table::Table for ActiveOrders {
    /// Key: OrderId
    type Key = B256;

    /// Value: 订单元数据
    type Value = OrderMetadata;

    const NAME: &'static str = "ActiveOrders";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderMetadata {
    /// 订单所有者
    pub owner: Address,

    /// 交易对ID
    pub symbol_id: u32,

    /// 市场类型（Spot/Perp）
    pub market_type: MarketType,

    /// 订单方向
    pub side: Side,

    /// 价格
    pub price: u64,

    /// 原始数量
    pub original_quantity: u64,

    /// 剩余数量
    pub remaining_quantity: u64,

    /// 创建区块
    pub created_at: u64,
}

/// 订单簿快照表（定期存储，用于快速恢复）
#[derive(Debug)]
pub struct OrderBookSnapshots;

impl table::Table for OrderBookSnapshots {
    /// Key: (SymbolId, BlockNumber)
    type Key = (u32, u64);

    /// Value: 订单簿快照
    type Value = OrderBookSnapshot;

    const NAME: &'static str = "OrderBookSnapshots";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderBookSnapshot {
    /// 买单簿（价格降序）
    pub bids: Vec<PriceLevel>,

    /// 卖单簿（价格升序）
    pub asks: Vec<PriceLevel>,

    /// 最新成交价
    pub last_price: u64,

    /// 快照时间
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: u64,
    pub quantity: u64,
    pub order_count: u32,
}

/// 成交历史表
#[derive(Debug)]
pub struct TradeHistory;

impl table::Table for TradeHistory {
    /// Key: (SymbolId, TradeId)
    type Key = (u32, u64);

    /// Value: 成交记录
    type Value = Trade;

    const NAME: &'static str = "TradeHistory";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trade {
    /// 买方
    pub buyer: Address,

    /// 卖方
    pub seller: Address,

    /// 成交价
    pub price: u64,

    /// 成交量
    pub quantity: u64,

    /// 成交时间
    pub timestamp: u64,

    /// 区块号
    pub block_number: u64,

    /// 买方订单ID
    pub buyer_order_id: B256,

    /// 卖方订单ID
    pub seller_order_id: B256,
}

/// 强平历史表
#[derive(Debug)]
pub struct LiquidationHistory;

impl table::Table for LiquidationHistory {
    /// Key: (Address, Timestamp)
    type Key = (Address, u64);

    /// Value: 强平记录
    type Value = LiquidationRecord;

    const NAME: &'static str = "LiquidationHistory";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiquidationRecord {
    /// 被强平用户
    pub user: Address,

    /// 交易对
    pub symbol_id: u32,

    /// 强平价格
    pub liquidation_price: u64,

    /// 强平数量
    pub quantity: u64,

    /// 亏损金额
    pub loss: u128,

    /// 强平费用
    pub fee: u128,

    /// 区块号
    pub block_number: u64,

    /// 强平类型（系统/ADL）
    pub liquidation_type: LiquidationType,
}
```

### 2.3 状态存储位置总结

```rust
/// 状态管理器：协调内存和持久化
pub struct DexStateManager {
    /// L0: 内存订单簿
    orderbooks: DashMap<u32, OrderBook>,

    /// L0: 持仓缓存
    position_cache: DashMap<(Address, u32), PerpPosition>,

    /// L1: MDBX数据库
    db: Arc<DatabaseEnv>,

    /// 写前日志（WAL）
    wal: WriteAheadLog,
}

impl DexStateManager {
    /// 查询Spot余额
    pub fn get_spot_balance(&self, user: Address, asset_id: u32) -> Result<SpotBalance> {
        // 1. 先查内存缓存（如果有）
        // 2. 查MDBX
        let tx = self.db.tx()?;
        let balance = tx.get::<SpotBalances>((user, asset_id))?;
        Ok(balance.unwrap_or_default())
    }

    /// 查询Perp持仓
    pub fn get_perp_position(&self, user: Address, symbol_id: u32) -> Result<Option<PerpPosition>> {
        // 1. 先查内存缓存（L0）
        if let Some(pos) = self.position_cache.get(&(user, symbol_id)) {
            return Ok(Some(pos.clone()));
        }

        // 2. 查MDBX（L1）
        let tx = self.db.tx()?;
        tx.get::<PerpPositions>((user, symbol_id))
    }

    /// 查询订单簿（只从内存）
    pub fn get_orderbook(&self, symbol_id: u32) -> Option<&OrderBook> {
        self.orderbooks.get(&symbol_id).map(|r| r.value())
    }

    /// 执行Spot交易（内存 + WAL）
    pub fn execute_spot_trade(&mut self, trade: &Trade) -> Result<()> {
        // 1. 更新内存订单簿
        let orderbook = self.orderbooks.get_mut(&trade.symbol_id).unwrap();
        orderbook.apply_trade(trade);

        // 2. 写WAL（保证崩溃恢复）
        self.wal.append(StateChange::SpotTrade(trade.clone()))?;

        // 3. 更新余额（内存 + 标记脏页）
        self.update_spot_balances(trade)?;

        Ok(())
    }

    /// 执行Perp交易（内存 + WAL）
    pub fn execute_perp_trade(&mut self, trade: &Trade) -> Result<()> {
        // 1. 更新内存订单簿
        let orderbook = self.orderbooks.get_mut(&trade.symbol_id).unwrap();
        orderbook.apply_trade(trade);

        // 2. 写WAL
        self.wal.append(StateChange::PerpTrade(trade.clone()))?;

        // 3. 更新持仓（内存缓存）
        self.update_positions(trade)?;

        Ok(())
    }

    /// 区块确认后：刷盘
    pub fn commit_block(&mut self, block_number: u64) -> Result<()> {
        let tx = self.db.tx_mut()?;

        // 1. 从WAL读取所有变更
        let changes = self.wal.read_uncommitted()?;

        // 2. 批量写入MDBX
        for change in changes {
            match change {
                StateChange::SpotBalance { user, asset_id, balance } => {
                    tx.put::<SpotBalances>((user, asset_id), balance)?;
                }
                StateChange::PerpPosition { user, symbol_id, position } => {
                    tx.put::<PerpPositions>((user, symbol_id), position)?;
                }
                StateChange::SpotTrade(trade) => {
                    tx.put::<TradeHistory>((trade.symbol_id, trade.trade_id), trade)?;
                }
                // ... 其他变更类型
            }
        }

        // 3. 提交事务
        tx.commit()?;

        // 4. 清除WAL
        self.wal.clear()?;

        // 5. 定期快照订单簿（每100块）
        if block_number % 100 == 0 {
            self.snapshot_orderbooks(block_number)?;
        }

        Ok(())
    }

    /// 快照订单簿到MDBX
    fn snapshot_orderbooks(&self, block_number: u64) -> Result<()> {
        let tx = self.db.tx_mut()?;

        for entry in self.orderbooks.iter() {
            let symbol_id = *entry.key();
            let orderbook = entry.value();

            let snapshot = OrderBookSnapshot {
                bids: orderbook.get_bids_snapshot(),
                asks: orderbook.get_asks_snapshot(),
                last_price: orderbook.last_price,
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            };

            tx.put::<OrderBookSnapshots>((symbol_id, block_number), snapshot)?;
        }

        tx.commit()
    }
}
```

### 2.4 状态根计算

```rust
/// 状态根计算：合并EVM和DEXVM
pub struct StateRootCalculator {
    evm_state: Arc<EvmState>,
    dex_state: Arc<DexStateManager>,
}

impl StateRootCalculator {
    /// 计算统一状态根
    pub fn compute_state_root(&self, block_number: u64) -> B256 {
        // 方案1：简单哈希合并（快速）
        self.compute_simple_root()

        // 方案2：统一MPT（兼容以太坊，但慢）
        // self.compute_mpt_root()
    }

    /// 方案1：简单哈希合并
    fn compute_simple_root(&self) -> B256 {
        // 1. EVM状态根（标准MPT）
        let evm_root = self.evm_state.state_root();

        // 2. DEXVM状态根（自定义）
        let dex_root = self.compute_dex_root();

        // 3. 合并
        keccak256(&[evm_root.as_slice(), dex_root.as_slice()].concat())
    }

    /// 计算DEXVM状态根
    fn compute_dex_root(&self) -> B256 {
        let mut hasher = Keccak256::new();

        // 1. Spot余额根
        let spot_root = self.compute_spot_balances_root();
        hasher.update(spot_root.as_slice());

        // 2. Perp持仓根
        let perp_root = self.compute_perp_positions_root();
        hasher.update(perp_root.as_slice());

        // 3. 订单簿根（可选，可以只用快照哈希）
        let orderbook_root = self.compute_orderbook_root();
        hasher.update(orderbook_root.as_slice());

        B256::from(hasher.finalize().as_slice())
    }

    /// 计算Spot余额根
    fn compute_spot_balances_root(&self) -> B256 {
        let tx = self.dex_state.db.tx().unwrap();
        let cursor = tx.cursor_read::<SpotBalances>().unwrap();

        let mut hasher = Keccak256::new();
        for entry in cursor.walk(None) {
            let ((user, asset_id), balance) = entry.unwrap();

            // 哈希：user || asset_id || available || frozen
            hasher.update(user.as_slice());
            hasher.update(&asset_id.to_be_bytes());
            hasher.update(&balance.available.to_be_bytes());
            hasher.update(&balance.frozen.to_be_bytes());
        }

        B256::from(hasher.finalize().as_slice())
    }

    /// 方案2：统一MPT（更复杂但兼容以太坊）
    fn compute_mpt_root(&self) -> B256 {
        // 构建统一的MPT，包含EVM和DEXVM状态
        let mut trie = PatriciaTrie::new();

        // 1. 插入EVM账户
        for (address, account) in self.evm_state.accounts() {
            let key = keccak256(address.as_slice());
            trie.insert(key.as_slice(), account.rlp_encode());
        }

        // 2. 插入DEXVM状态（使用特殊前缀避免冲突）
        let dex_prefix = b"DEX:";

        // 2.1 Spot余额
        for ((user, asset_id), balance) in self.dex_state.iter_spot_balances() {
            let key = keccak256(&[dex_prefix, b"SPOT:", user.as_slice(), &asset_id.to_be_bytes()].concat());
            trie.insert(key.as_slice(), balance.rlp_encode());
        }

        // 2.2 Perp持仓
        for ((user, symbol_id), position) in self.dex_state.iter_perp_positions() {
            let key = keccak256(&[dex_prefix, b"PERP:", user.as_slice(), &symbol_id.to_be_bytes()].concat());
            trie.insert(key.as_slice(), position.rlp_encode());
        }

        // 3. 计算根
        trie.root()
    }
}
```

## 3. 关键问题解答

### 3.1 DEXVM状态写在哪里？

**答案：分层存储**

```
L0 内存（微秒级）
├─ 订单簿：BTreeMap + DashMap（完全在内存）
├─ 持仓缓存：DashMap（热数据）
└─ 价格缓存：最新成交价

L1 MDBX热表（毫秒级）
├─ SpotBalances：用户余额
├─ PerpPositions：持仓详情
├─ PerpMargins：保证金账户
└─ ActiveOrders：未成交订单索引

L2 MDBX归档表（秒级）
├─ TradeHistory：成交历史
├─ OrderHistory：订单历史
└─ LiquidationHistory：强平记录

L3 Static Files（分钟级）
└─ 历史数据归档

写入流程：
1. 交易执行 → 内存 + WAL（立即）
2. 区块确认 → MDBX热表（~500ms）
3. 定期归档 → MDBX归档表（~10分钟）
4. 冷数据迁移 → Static Files（~24小时）
```

### 3.2 EVM和DEXVM状态如何协调？

**答案：物理分离，逻辑关联**

```rust
// 物理分离：不同的MDBX表
EVM Tables (Reth标准):
├─ AccountsHistory
├─ StorageHistory
├─ PlainAccountState
└─ PlainStorageState

DEXVM Tables (新增):
├─ SpotBalances
├─ PerpPositions
├─ PerpMargins
└─ ActiveOrders

// 逻辑关联：通过用户地址
User Address → {
    EVM: Account { balance, nonce, code, storage }
    Spot: { BTC: 1.5, USDT: 100000 }
    Perp: { margin: 50000, positions: [...] }
}

// 状态根：合并计算
StateRoot = H(
    EVM_State_Root,
    DEX_State_Root
)
```

### 3.3 区块验证流程

```rust
/// 区块验证器
pub struct DexBlockValidator {
    evm_validator: EvmBlockValidator,
    dex_validator: DexBlockValidator,
}

impl DexBlockValidator {
    /// 验证区块
    pub fn validate_block(&self, block: &DexBlock) -> Result<()> {
        // 1. 并行验证EVM和DEXVM部分
        let (evm_result, dex_result) = rayon::join(
            || self.validate_evm_part(block),
            || self.validate_dex_part(block),
        );

        evm_result?;
        dex_result?;

        // 2. 验证状态根
        self.verify_state_root(block)?;

        // 3. 验证交易根
        self.verify_transactions_root(block)?;

        Ok(())
    }

    /// 验证EVM部分
    fn validate_evm_part(&self, block: &DexBlock) -> Result<()> {
        // 标准EVM验证
        self.evm_validator.validate(&block.evm_body)?;
        Ok(())
    }

    /// 验证DEXVM部分
    fn validate_dex_part(&self, block: &DexBlock) -> Result<()> {
        // 1. 验证签名
        for tx in &block.dex_body.spot_txs {
            self.verify_signature(tx)?;
        }

        for tx in &block.dex_body.perp_txs {
            self.verify_signature(tx)?;
        }

        // 2. 重新执行撮合，验证确定性
        let execution_result = self.execute_dex_transactions(&block.dex_body)?;

        // 3. 验证成交结果
        self.verify_execution_result(&execution_result)?;

        Ok(())
    }
}
```

## 4. 性能优化建议

```rust
/// 性能优化清单
pub struct PerformanceOptimizations;

impl PerformanceOptimizations {
    /// 1. 紧凑编码
    /// - Spot交易：120字节 vs EVM交易：~200字节
    /// - Perp交易：125字节
    /// - 节省37.5%空间

    /// 2. 批量交易
    /// - 多个订单共享一个签名
    /// - 节省64字节 * N
    /// - 适合做市商、策略

    /// 3. 订单簿内存优化
    /// - 使用内存池避免频繁分配
    /// - 订单ID用u64代替B256（内存中）
    /// - 价格档位预分配

    /// 4. 并行处理
    /// - EVM和DEXVM交易并行验证
    /// - 不同交易对的订单并行撮合
    /// - 状态根并行计算

    /// 5. 写入优化
    /// - WAL批量刷盘
    /// - MDBX写事务合并
    /// - 订单簿异步快照

    /// 6. 缓存策略
    /// - 持仓缓存（热数据）
    /// - 价格缓存（避免重复查询）
    /// - 订单簿快照（定期持久化）
}
```

## 5. 总结

### 5.1 核心设计决策

| 方面 | 决策 | 理由 |
|------|------|------|
| **区块结构** | Reth兼容 + DEXVM扩展 | 保持兼容性，清晰分离 |
| **交易编码** | 紧凑二进制格式 | 节省空间，提升吞吐 |
| **状态存储** | 分层：内存 + MDBX + Static Files | 平衡性能和持久化 |
| **状态根** | 简单哈希合并 | 快速计算，满足验证需求 |
| **表设计** | 物理分离，逻辑关联 | 独立扩展，清晰职责 |

### 5.2 容量与性能

```
区块配置：
- 大小限制：10 MB
- 出块时间：500 ms
- EVM gas：30M

容量估算：
- EVM交易：~600笔/块
- DEXVM交易：~82,000笔/块
- 总TPS：165,000 ✅

状态大小（100万用户）：
- Spot余额：100万 * 40字节 = 40 MB
- Perp持仓：10万 * 80字节 = 8 MB
- 订单簿（内存）：1000交易对 * 1MB = 1 GB
- 总计：~1.05 GB（可接受）
```

### 5.3 实施建议

**Phase 1：基础实现**
```
1. 实现DexBlock和DexTransaction结构
2. 扩展MDBX表定义
3. 实现基本的状态读写
4. 简单的状态根计算
```

**Phase 2：性能优化**
```
1. 内存订单簿优化
2. WAL和批量写入
3. 并行验证和执行
4. 缓存策略
```

**Phase 3：生产就绪**
```
1. 完整的崩溃恢复
2. 状态裁剪和归档
3. 监控和指标
4. 压力测试
```
