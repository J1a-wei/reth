//! DexVM 基础数据类型
//!
//! 包含订单、账户、交易对等核心数据结构

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod account;
pub mod instruction;
pub mod order;
pub mod transaction;
pub mod trading_pair;

pub use account::{DexAccount, DexAccountState};
pub use instruction::DexInstruction;
pub use order::{Order, OrderId, OrderSide, OrderStatus, OrderType};
pub use transaction::{DexTransaction, SignedDexTransaction};
pub use trading_pair::TradingPair;

use alloy_primitives::{Address, B256, U256};
use thiserror::Error;

/// DexVM 执行结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    /// 是否成功
    pub success: bool,
    /// Gas 消耗
    pub gas_used: u64,
    /// 返回数据
    pub output: ExecutionOutput,
}

/// 执行输出
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionOutput {
    /// 下单成功，返回订单 ID
    OrderPlaced(OrderId),
    /// 订单取消成功
    OrderCancelled(OrderId),
    /// 订单成交记录
    TradesExecuted(Vec<Trade>),
    /// 查询结果（余额、订单簿等）
    QueryResult(Vec<u8>),
    /// 错误信息
    Error(String),
}

/// 成交记录
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Trade {
    /// 交易 ID
    pub trade_id: B256,
    /// 买单 ID
    pub buy_order_id: OrderId,
    /// 卖单 ID
    pub sell_order_id: OrderId,
    /// 交易对
    pub pair: TradingPair,
    /// 成交价格
    pub price: U256,
    /// 成交数量
    pub amount: U256,
    /// 买方地址
    pub buyer: Address,
    /// 卖方地址
    pub seller: Address,
    /// 时间戳
    pub timestamp: u64,
}

/// DexVM 错误
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DexVmError {
    /// 余额不足
    #[error("Insufficient balance: have {have}, need {need}")]
    InsufficientBalance { have: U256, need: U256 },

    /// 订单不存在
    #[error("Order not found: {0:?}")]
    OrderNotFound(OrderId),

    /// 无效的订单参数
    #[error("Invalid order: {0}")]
    InvalidOrder(String),

    /// 无效的签名
    #[error("Invalid signature")]
    InvalidSignature,

    /// Nonce 错误
    #[error("Invalid nonce: expected {expected}, got {got}")]
    InvalidNonce { expected: u64, got: u64 },

    /// Gas 不足
    #[error("Out of gas")]
    OutOfGas,

    /// 内部错误
    #[error("Internal error: {0}")]
    Internal(String),
}

/// DexVM 执行统计
#[derive(Debug, Default, Clone)]
pub struct ExecutionStats {
    /// 处理的交易数
    pub transactions_processed: u64,
    /// 下单数量
    pub orders_placed: u64,
    /// 取消数量
    pub orders_cancelled: u64,
    /// 成交数量
    pub trades_executed: u64,
    /// 总 Gas 消耗
    pub total_gas_used: u64,
}
