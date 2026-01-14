//! DexVM 指令集

use crate::{OrderId, OrderSide, TradingPair};
use alloy_primitives::{Address, U256};
use alloy_rlp::{Encodable, Decodable, BufMut};
use serde::{Deserialize, Serialize};

/// DexVM 指令
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DexInstruction {
    /// 下限价单
    PlaceLimitOrder {
        /// 交易对
        pair: TradingPair,
        /// 订单方向
        side: OrderSide,
        /// 价格
        price: U256,
        /// 数量
        amount: U256,
    },

    /// 取消订单
    CancelOrder {
        /// 订单 ID
        order_id: OrderId,
    },

    /// 充值
    Deposit {
        /// 代币地址
        token: Address,
        /// 数量
        amount: U256,
    },

    /// 提现
    Withdraw {
        /// 代币地址
        token: Address,
        /// 数量
        amount: U256,
    },

    /// 查询订单簿
    QueryOrderBook {
        /// 交易对
        pair: TradingPair,
        /// 深度（每边返回多少档）
        depth: u32,
    },

    /// 查询账户余额
    QueryBalance {
        /// 代币地址
        token: Address,
    },

    /// 查询订单状态
    QueryOrder {
        /// 订单 ID
        order_id: OrderId,
    },

    /// 默认值（空操作）
    #[default]
    Noop,
}

impl DexInstruction {
    /// 估算 Gas 消耗
    pub fn estimate_gas(&self) -> u64 {
        match self {
            DexInstruction::PlaceLimitOrder { .. } => 50_000,
            DexInstruction::CancelOrder { .. } => 30_000,
            DexInstruction::Deposit { .. } => 20_000,
            DexInstruction::Withdraw { .. } => 20_000,
            DexInstruction::QueryOrderBook { .. } => 10_000,
            DexInstruction::QueryBalance { .. } => 5_000,
            DexInstruction::QueryOrder { .. } => 5_000,
            DexInstruction::Noop => 0,
        }
    }

    /// 是否只读操作
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            DexInstruction::QueryOrderBook { .. }
                | DexInstruction::QueryBalance { .. }
                | DexInstruction::QueryOrder { .. }
        )
    }
}

impl Encodable for DexInstruction {
    fn encode(&self, out: &mut dyn BufMut) {
        // Simple encoding: instruction type (u8) + fields
        match self {
            DexInstruction::PlaceLimitOrder { pair, side, price, amount } => {
                0u8.encode(out);
                pair.encode(out);
                (*side as u8).encode(out);
                price.encode(out);
                amount.encode(out);
            }
            DexInstruction::CancelOrder { order_id } => {
                1u8.encode(out);
                order_id.encode(out);
            }
            DexInstruction::Deposit { token, amount } => {
                2u8.encode(out);
                token.encode(out);
                amount.encode(out);
            }
            DexInstruction::Withdraw { token, amount } => {
                3u8.encode(out);
                token.encode(out);
                amount.encode(out);
            }
            DexInstruction::QueryOrderBook { pair, depth } => {
                4u8.encode(out);
                pair.encode(out);
                depth.encode(out);
            }
            DexInstruction::QueryBalance { token } => {
                5u8.encode(out);
                token.encode(out);
            }
            DexInstruction::QueryOrder { order_id } => {
                6u8.encode(out);
                order_id.encode(out);
            }
            DexInstruction::Noop => {
                7u8.encode(out);
            }
        }
    }

    fn length(&self) -> usize {
        match self {
            DexInstruction::PlaceLimitOrder { pair, side, price, amount } => {
                1 + pair.length() + 1 + price.length() + amount.length()
            }
            DexInstruction::CancelOrder { order_id } => 1 + order_id.length(),
            DexInstruction::Deposit { token, amount } => 1 + token.length() + amount.length(),
            DexInstruction::Withdraw { token, amount } => 1 + token.length() + amount.length(),
            DexInstruction::QueryOrderBook { pair, depth } => 1 + pair.length() + depth.length(),
            DexInstruction::QueryBalance { token } => 1 + token.length(),
            DexInstruction::QueryOrder { order_id } => 1 + order_id.length(),
            DexInstruction::Noop => 1,
        }
    }
}

impl Decodable for DexInstruction {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let tag = u8::decode(buf)?;
        Ok(match tag {
            0 => {
                let pair = TradingPair::decode(buf)?;
                let side_byte = u8::decode(buf)?;
                let side = if side_byte == 0 { OrderSide::Buy } else { OrderSide::Sell };
                let price = U256::decode(buf)?;
                let amount = U256::decode(buf)?;
                DexInstruction::PlaceLimitOrder { pair, side, price, amount }
            }
            1 => {
                let order_id = OrderId::decode(buf)?;
                DexInstruction::CancelOrder { order_id }
            }
            2 => {
                let token = Address::decode(buf)?;
                let amount = U256::decode(buf)?;
                DexInstruction::Deposit { token, amount }
            }
            3 => {
                let token = Address::decode(buf)?;
                let amount = U256::decode(buf)?;
                DexInstruction::Withdraw { token, amount }
            }
            4 => {
                let pair = TradingPair::decode(buf)?;
                let depth = u32::decode(buf)?;
                DexInstruction::QueryOrderBook { pair, depth }
            }
            5 => {
                let token = Address::decode(buf)?;
                DexInstruction::QueryBalance { token }
            }
            6 => {
                let order_id = OrderId::decode(buf)?;
                DexInstruction::QueryOrder { order_id }
            }
            7 => DexInstruction::Noop,
            _ => return Err(alloy_rlp::Error::Custom("Invalid instruction tag")),
        })
    }
}
