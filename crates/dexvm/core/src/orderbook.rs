//! 高性能订单簿实现
//!
//! 使用 BTreeMap 实现价格优先、时间优先的订单簿

use dashmap::DashMap;
use reth_dexvm_primitives::{Order, OrderId, OrderSide, OrderStatus, Trade, TradingPair};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use alloy_primitives::{Address, B256, U256};

/// 价格档位（同一价格的订单队列）
#[derive(Debug, Clone)]
pub struct PriceLevel {
    /// 价格
    pub price: U256,
    /// 订单队列（时间优先）
    pub orders: VecDeque<OrderId>,
    /// 总数量
    pub total_amount: U256,
}

impl PriceLevel {
    fn new(price: U256) -> Self {
        Self {
            price,
            orders: VecDeque::new(),
            total_amount: U256::ZERO,
        }
    }

    fn add_order(&mut self, order_id: OrderId, amount: U256) {
        self.orders.push_back(order_id);
        self.total_amount += amount;
    }

    fn remove_order(&mut self, order_id: &OrderId, amount: U256) -> bool {
        if let Some(pos) = self.orders.iter().position(|id| id == order_id) {
            self.orders.remove(pos);
            self.total_amount = self.total_amount.saturating_sub(amount);
            true
        } else {
            false
        }
    }

    fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }
}

/// 单边订单簿（买单或卖单）
#[derive(Debug)]
struct OrderBookSide {
    /// 价格 -> 价格档位
    levels: BTreeMap<U256, PriceLevel>,
    /// 是否买单
    is_buy: bool,
}

impl OrderBookSide {
    fn new(is_buy: bool) -> Self {
        Self {
            levels: BTreeMap::new(),
            is_buy,
        }
    }

    fn add_order(&mut self, order: &Order) {
        let level = self.levels.entry(order.price).or_insert_with(|| PriceLevel::new(order.price));
        level.add_order(order.id, order.remaining);
    }

    fn remove_order(&mut self, price: U256, order_id: &OrderId, amount: U256) {
        if let Some(level) = self.levels.get_mut(&price) {
            level.remove_order(order_id, amount);
            if level.is_empty() {
                self.levels.remove(&price);
            }
        }
    }

    fn best_price(&self) -> Option<U256> {
        if self.is_buy {
            // 买单：最高价
            self.levels.keys().next_back().copied()
        } else {
            // 卖单：最低价
            self.levels.keys().next().copied()
        }
    }

    fn get_level(&self, price: &U256) -> Option<&PriceLevel> {
        self.levels.get(price)
    }

    fn iter_levels(&self) -> impl Iterator<Item = (&U256, &PriceLevel)> {
        if self.is_buy {
            // 买单：从高到低
            Box::new(self.levels.iter().rev()) as Box<dyn Iterator<Item = _>>
        } else {
            // 卖单：从低到高
            Box::new(self.levels.iter())
        }
    }
}

/// 订单簿
pub struct OrderBook {
    /// 交易对
    pub pair: TradingPair,
    /// 买单簿
    bids: OrderBookSide,
    /// 卖单簿
    asks: OrderBookSide,
    /// 所有订单 (order_id -> order)
    orders: DashMap<OrderId, Order>,
    /// 最新成交价
    last_price: parking_lot::RwLock<Option<U256>>,
}

impl OrderBook {
    /// 创建新订单簿
    pub fn new(pair: TradingPair) -> Self {
        Self {
            pair,
            bids: OrderBookSide::new(true),
            asks: OrderBookSide::new(false),
            orders: DashMap::new(),
            last_price: parking_lot::RwLock::new(None),
        }
    }

    /// 添加订单
    pub fn add_order(&mut self, order: Order) {
        match order.side {
            OrderSide::Buy => self.bids.add_order(&order),
            OrderSide::Sell => self.asks.add_order(&order),
        }
        self.orders.insert(order.id, order);
    }

    /// 取消订单
    pub fn cancel_order(&mut self, order_id: &OrderId) -> Option<Order> {
        if let Some(mut entry) = self.orders.get_mut(order_id) {
            let order = entry.value_mut();
            if order.status != OrderStatus::Active {
                return None;
            }

            let price = order.price;
            let amount = order.remaining;
            let side = order.side;

            order.cancel();

            match side {
                OrderSide::Buy => self.bids.remove_order(price, order_id, amount),
                OrderSide::Sell => self.asks.remove_order(price, order_id, amount),
            }

            Some(order.clone())
        } else {
            None
        }
    }

    /// 获取订单
    pub fn get_order(&self, order_id: &OrderId) -> Option<Order> {
        self.orders.get(order_id).map(|r| r.value().clone())
    }

    /// 获取最佳买价
    pub fn best_bid(&self) -> Option<U256> {
        self.bids.best_price()
    }

    /// 获取最佳卖价
    pub fn best_ask(&self) -> Option<U256> {
        self.asks.best_price()
    }

    /// 获取最新成交价
    pub fn last_price(&self) -> Option<U256> {
        *self.last_price.read()
    }

    /// 更新最新成交价
    fn update_last_price(&self, price: U256) {
        *self.last_price.write() = Some(price);
    }

    /// 获取订单簿深度
    pub fn get_depth(&self, depth: usize) -> (Vec<(U256, U256)>, Vec<(U256, U256)>) {
        let bids: Vec<_> = self
            .bids
            .iter_levels()
            .take(depth)
            .map(|(price, level)| (*price, level.total_amount))
            .collect();

        let asks: Vec<_> = self
            .asks
            .iter_levels()
            .take(depth)
            .map(|(price, level)| (*price, level.total_amount))
            .collect();

        (bids, asks)
    }

    /// 匹配订单（核心撮合逻辑）
    pub fn match_order(&mut self, mut taker_order: Order, timestamp: u64) -> Vec<Trade> {
        let mut trades = Vec::new();
        let mut last_traded_price = None;

        let taker_is_buy = taker_order.side == OrderSide::Buy;

        // 持续撮合直到完全成交或无法匹配
        while taker_order.remaining > U256::ZERO {
            let opposite_side = match taker_order.side {
                OrderSide::Buy => &mut self.asks,
                OrderSide::Sell => &mut self.bids,
            };

            let best_price = opposite_side.best_price();
            if best_price.is_none() {
                break;
            }
            let best_price = best_price.unwrap();

            // 检查价格是否匹配
            let can_match = if taker_is_buy {
                taker_order.price >= best_price // 买单价格 >= 卖单价格
            } else {
                taker_order.price <= best_price // 卖单价格 <= 买单价格
            };

            if !can_match {
                break;
            }

            // 获取最优价格的订单队列
            let maker_order_ids: Vec<OrderId> = {
                let level = opposite_side.get_level(&best_price);
                if level.is_none() {
                    break;
                }
                level.unwrap().orders.iter().copied().collect()
            };

            for maker_order_id in maker_order_ids {
                if taker_order.remaining.is_zero() {
                    break;
                }

                let mut maker_entry = match self.orders.get_mut(&maker_order_id) {
                    Some(entry) => entry,
                    None => continue,
                };

                let maker_order = maker_entry.value_mut();
                if maker_order.status != OrderStatus::Active {
                    continue;
                }

                // 计算成交数量
                let fill_amount = taker_order.remaining.min(maker_order.remaining);
                let trade_price = maker_order.price; // 使用挂单价格

                // 更新订单状态
                taker_order.fill(fill_amount).ok();
                maker_order.fill(fill_amount).ok();

                // Record trade details before updating orderbook
                let (buyer, seller) = if taker_is_buy {
                    (taker_order.maker, maker_order.maker)
                } else {
                    (maker_order.maker, taker_order.maker)
                };

                last_traded_price = Some(trade_price);

                // Drop the maker_entry reference before modifying opposite_side
                drop(maker_entry);

                // Update orderbook
                let opposite_side = match taker_order.side {
                    OrderSide::Buy => &mut self.asks,
                    OrderSide::Sell => &mut self.bids,
                };

                // Get maker order status again to check if filled
                let maker_filled = self.orders.get(&maker_order_id)
                    .map(|o| o.is_filled())
                    .unwrap_or(false);

                if maker_filled {
                    opposite_side.remove_order(best_price, &maker_order_id, U256::ZERO);
                } else {
                    if let Some(level) = opposite_side.levels.get_mut(&best_price) {
                        level.total_amount -= fill_amount;
                    }
                }

                trades.push(Trade {
                    trade_id: B256::left_padding_from(&timestamp.to_le_bytes()),
                    buy_order_id: if taker_is_buy { taker_order.id } else { maker_order_id },
                    sell_order_id: if taker_is_buy { maker_order_id } else { taker_order.id },
                    pair: self.pair,
                    price: trade_price,
                    amount: fill_amount,
                    buyer,
                    seller,
                    timestamp,
                });
            }
        }

        // Update last price after all trades
        if let Some(price) = last_traded_price {
            self.update_last_price(price);
        }

        // 如果 taker 订单未完全成交，加入订单簿
        if taker_order.remaining > U256::ZERO && taker_order.status == OrderStatus::Active {
            self.add_order(taker_order);
        }

        trades
    }

    /// 订单数量
    pub fn order_count(&self) -> usize {
        self.orders.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    fn create_test_pair() -> TradingPair {
        TradingPair::new(
            address!("0000000000000000000000000000000000000001"), // BTC
            address!("0000000000000000000000000000000000000002"), // USDT
        )
    }

    #[test]
    fn test_orderbook_add_order() {
        let pair = create_test_pair();
        let mut book = OrderBook::new(pair);

        let order = Order::new(
            B256::ZERO,
            pair,
            address!("0000000000000000000000000000000000000003"),
            OrderSide::Buy,
            U256::from(50000),
            U256::from(1000),
            0,
        );

        book.add_order(order.clone());
        assert_eq!(book.order_count(), 1);
        assert_eq!(book.best_bid(), Some(U256::from(50000)));
    }

    #[test]
    fn test_orderbook_matching() {
        let pair = create_test_pair();
        let mut book = OrderBook::new(pair);

        // 添加卖单
        let sell_order = Order::new(
            B256::from([1u8; 32]),
            pair,
            address!("0000000000000000000000000000000000000003"),
            OrderSide::Sell,
            U256::from(50000),
            U256::from(500),
            0,
        );
        book.add_order(sell_order);

        // 添加买单进行撮合
        let buy_order = Order::new(
            B256::from([2u8; 32]),
            pair,
            address!("0000000000000000000000000000000000000004"),
            OrderSide::Buy,
            U256::from(50000),
            U256::from(300),
            1,
        );

        let trades = book.match_order(buy_order, 1);
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].amount, U256::from(300));
        assert_eq!(trades[0].price, U256::from(50000));
    }

    #[test]
    fn test_orderbook_cancel() {
        let pair = create_test_pair();
        let mut book = OrderBook::new(pair);

        let order = Order::new(
            B256::ZERO,
            pair,
            address!("0000000000000000000000000000000000000003"),
            OrderSide::Buy,
            U256::from(50000),
            U256::from(1000),
            0,
        );

        let order_id = order.id;
        book.add_order(order);

        let cancelled = book.cancel_order(&order_id);
        assert!(cancelled.is_some());
        assert_eq!(cancelled.unwrap().status, OrderStatus::Cancelled);
        assert_eq!(book.best_bid(), None);
    }
}
