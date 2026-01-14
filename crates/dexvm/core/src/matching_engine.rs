//! 撮合引擎
//!
//! 管理多个交易对的订单簿

use crate::orderbook::OrderBook;
use dashmap::DashMap;
use reth_dexvm_primitives::{Order, OrderId, Trade, TradingPair};
use std::sync::Arc;

/// 撮合引擎
pub struct MatchingEngine {
    /// 交易对 -> 订单簿
    orderbooks: DashMap<TradingPair, OrderBook>,
}

impl MatchingEngine {
    /// 创建新引擎
    pub fn new() -> Self {
        Self {
            orderbooks: DashMap::new(),
        }
    }

    /// 获取或创建订单簿
    fn get_or_create_orderbook(&self, pair: TradingPair) -> dashmap::mapref::one::RefMut<TradingPair, OrderBook> {
        self.orderbooks.entry(pair).or_insert_with(|| OrderBook::new(pair))
    }

    /// 下单
    pub fn place_order(&self, order: Order, timestamp: u64) -> Vec<Trade> {
        let mut book = self.get_or_create_orderbook(order.pair);
        book.match_order(order, timestamp)
    }

    /// 取消订单
    pub fn cancel_order(&self, pair: TradingPair, order_id: &OrderId) -> Option<Order> {
        self.orderbooks.get_mut(&pair).and_then(|mut book| book.cancel_order(order_id))
    }

    /// 获取订单
    pub fn get_order(&self, pair: TradingPair, order_id: &OrderId) -> Option<Order> {
        self.orderbooks.get(&pair).and_then(|book| book.get_order(order_id))
    }

    /// 获取订单簿深度
    pub fn get_depth(&self, pair: TradingPair, depth: usize) -> Option<(Vec<(alloy_primitives::U256, alloy_primitives::U256)>, Vec<(alloy_primitives::U256, alloy_primitives::U256)>)> {
        self.orderbooks.get(&pair).map(|book| book.get_depth(depth))
    }

    /// 获取最新价格
    pub fn get_last_price(&self, pair: TradingPair) -> Option<alloy_primitives::U256> {
        self.orderbooks.get(&pair).and_then(|book| book.last_price())
    }

    /// 统计信息
    pub fn stats(&self) -> MatchingEngineStats {
        let mut total_orders = 0;
        let orderbook_count = self.orderbooks.len();

        for book in self.orderbooks.iter() {
            total_orders += book.value().order_count();
        }

        MatchingEngineStats {
            orderbook_count,
            total_orders,
        }
    }
}

impl Default for MatchingEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// 撮合引擎统计信息
#[derive(Debug, Clone)]
pub struct MatchingEngineStats {
    pub orderbook_count: usize,
    pub total_orders: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use reth_dexvm_primitives::{OrderSide, TradingPair};
    use alloy_primitives::{address, B256, U256};

    #[test]
    fn test_matching_engine() {
        let engine = MatchingEngine::new();
        let pair = TradingPair::new(
            address!("0000000000000000000000000000000000000001"),
            address!("0000000000000000000000000000000000000002"),
        );

        let sell_order = Order::new(
            B256::from([1u8; 32]),
            pair,
            address!("0000000000000000000000000000000000000003"),
            OrderSide::Sell,
            U256::from(50000),
            U256::from(500),
            0,
        );

        let trades = engine.place_order(sell_order, 0);
        assert_eq!(trades.len(), 0); // 无对手盘

        let buy_order = Order::new(
            B256::from([2u8; 32]),
            pair,
            address!("0000000000000000000000000000000000000004"),
            OrderSide::Buy,
            U256::from(50000),
            U256::from(300),
            1,
        );

        let trades = engine.place_order(buy_order, 1);
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].amount, U256::from(300));
    }
}
