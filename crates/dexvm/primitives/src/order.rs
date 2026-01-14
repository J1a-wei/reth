//! 订单数据结构

use crate::trading_pair::TradingPair;
use alloy_primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};

/// 订单 ID
pub type OrderId = B256;

/// 订单方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderSide {
    /// 买单
    Buy,
    /// 卖单
    Sell,
}

/// 订单类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    /// 限价单
    Limit,
    /// 市价单（未来扩展）
    Market,
}

/// 订单状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    /// 活跃（部分成交或未成交）
    Active,
    /// 完全成交
    Filled,
    /// 已取消
    Cancelled,
}

/// 订单
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    /// 订单 ID
    pub id: OrderId,
    /// 交易对
    pub pair: TradingPair,
    /// 下单用户
    pub maker: Address,
    /// 订单方向
    pub side: OrderSide,
    /// 订单类型
    pub order_type: OrderType,
    /// 价格（限价单）
    pub price: U256,
    /// 原始数量
    pub amount: U256,
    /// 剩余数量
    pub remaining: U256,
    /// 状态
    pub status: OrderStatus,
    /// 创建时间
    pub timestamp: u64,
}

impl Order {
    /// 创建新订单
    pub fn new(
        id: OrderId,
        pair: TradingPair,
        maker: Address,
        side: OrderSide,
        price: U256,
        amount: U256,
        timestamp: u64,
    ) -> Self {
        Self {
            id,
            pair,
            maker,
            side,
            order_type: OrderType::Limit,
            price,
            amount,
            remaining: amount,
            status: OrderStatus::Active,
            timestamp,
        }
    }

    /// 是否完全成交
    pub fn is_filled(&self) -> bool {
        self.remaining.is_zero()
    }

    /// 部分成交
    pub fn fill(&mut self, amount: U256) -> Result<(), String> {
        if amount > self.remaining {
            return Err(format!(
                "Fill amount {} exceeds remaining {}",
                amount, self.remaining
            ));
        }
        self.remaining -= amount;
        if self.remaining.is_zero() {
            self.status = OrderStatus::Filled;
        }
        Ok(())
    }

    /// 取消订单
    pub fn cancel(&mut self) {
        self.status = OrderStatus::Cancelled;
    }

    /// 是否买单
    pub fn is_buy(&self) -> bool {
        matches!(self.side, OrderSide::Buy)
    }

    /// 是否卖单
    pub fn is_sell(&self) -> bool {
        matches!(self.side, OrderSide::Sell)
    }

    /// 价格优先级比较（买单价格越高优先级越高，卖单价格越低优先级越高）
    pub fn price_priority(&self, other: &Order) -> std::cmp::Ordering {
        match self.side {
            OrderSide::Buy => other.price.cmp(&self.price), // 买单：价格高的优先
            OrderSide::Sell => self.price.cmp(&other.price), // 卖单：价格低的优先
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn test_order_fill() {
        let id = B256::ZERO;
        let pair = TradingPair::new(
            address!("0000000000000000000000000000000000000001"),
            address!("0000000000000000000000000000000000000002"),
        );
        let maker = address!("0000000000000000000000000000000000000003");
        let mut order = Order::new(
            id,
            pair,
            maker,
            OrderSide::Buy,
            U256::from(100),
            U256::from(1000),
            0,
        );

        assert_eq!(order.status, OrderStatus::Active);
        assert_eq!(order.remaining, U256::from(1000));

        order.fill(U256::from(300)).unwrap();
        assert_eq!(order.remaining, U256::from(700));
        assert_eq!(order.status, OrderStatus::Active);

        order.fill(U256::from(700)).unwrap();
        assert_eq!(order.remaining, U256::ZERO);
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_price_priority() {
        let pair = TradingPair::new(
            address!("0000000000000000000000000000000000000001"),
            address!("0000000000000000000000000000000000000002"),
        );
        let maker = address!("0000000000000000000000000000000000000003");

        let buy_order1 = Order::new(
            B256::ZERO,
            pair,
            maker,
            OrderSide::Buy,
            U256::from(100),
            U256::from(1000),
            0,
        );
        let buy_order2 = Order::new(
            B256::ZERO,
            pair,
            maker,
            OrderSide::Buy,
            U256::from(110),
            U256::from(1000),
            0,
        );

        // 买单：价格更高的优先
        assert_eq!(
            buy_order2.price_priority(&buy_order1),
            std::cmp::Ordering::Less
        );
    }
}
