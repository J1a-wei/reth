//! 交易对定义

use alloy_primitives::Address;
use alloy_rlp::{Encodable, Decodable, BufMut};
use serde::{Deserialize, Serialize};

/// 交易对
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TradingPair {
    /// 基础资产（如 BTC）
    pub base: Address,
    /// 计价资产（如 USDT）
    pub quote: Address,
}

impl TradingPair {
    /// 创建交易对
    pub const fn new(base: Address, quote: Address) -> Self {
        Self { base, quote }
    }

    /// 获取对应的代币地址（根据订单方向）
    pub fn get_token_for_side(&self, is_buy: bool) -> Address {
        if is_buy {
            self.quote // 买单需要计价币
        } else {
            self.base // 卖单需要基础币
        }
    }
}

impl std::fmt::Display for TradingPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}/{:?}", self.base, self.quote)
    }
}

impl Encodable for TradingPair {
    fn encode(&self, out: &mut dyn BufMut) {
        self.base.encode(out);
        self.quote.encode(out);
    }

    fn length(&self) -> usize {
        self.base.length() + self.quote.length()
    }
}

impl Decodable for TradingPair {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let base = Address::decode(buf)?;
        let quote = Address::decode(buf)?;
        Ok(TradingPair { base, quote })
    }
}
