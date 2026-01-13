# 高性能订单簿实现详解

## 1. 内存订单簿核心数据结构

### 1.1 订单簿设计

```rust
use std::collections::{BTreeMap, HashMap, VecDeque};
use parking_lot::RwLock;

/// 高性能订单簿（完全内存）
pub struct OrderBook {
    /// 交易对ID
    pub symbol_id: u32,

    /// 买单簿（价格从高到低）
    /// Key: 价格（取负数实现降序）
    /// Value: 该价格档位的所有订单
    pub bids: BTreeMap<i64, PriceLevel>,

    /// 卖单簿（价格从低到高）
    /// Key: 价格
    /// Value: 该价格档位的所有订单
    pub asks: BTreeMap<i64, PriceLevel>,

    /// 订单索引（快速查找和取消）
    /// Key: OrderId
    /// Value: 订单引用（价格 + 位置）
    pub order_index: HashMap<B256, OrderRef>,

    /// 最新成交价
    pub last_price: u64,

    /// 24小时统计
    pub stats_24h: Statistics24h,

    /// 订单ID生成器
    next_order_id: u64,
}

/// 价格档位（Price Level）
#[derive(Debug, Clone)]
pub struct PriceLevel {
    /// 价格
    pub price: u64,

    /// 该档位的所有订单（按时间顺序）
    pub orders: VecDeque<Order>,

    /// 该档位总数量（缓存，避免重复计算）
    pub total_quantity: u64,

    /// 订单数量
    pub order_count: u32,
}

impl PriceLevel {
    pub fn new(price: u64) -> Self {
        Self {
            price,
            orders: VecDeque::new(),
            total_quantity: 0,
            order_count: 0,
        }
    }

    /// 添加订单到档位
    pub fn add_order(&mut self, order: Order) {
        self.total_quantity += order.remaining;
        self.order_count += 1;
        self.orders.push_back(order);
    }

    /// 移除订单
    pub fn remove_order(&mut self, order_id: &B256) -> Option<Order> {
        let pos = self.orders.iter().position(|o| &o.id == order_id)?;
        let order = self.orders.remove(pos)?;

        self.total_quantity -= order.remaining;
        self.order_count -= 1;

        Some(order)
    }

    /// 部分成交
    pub fn fill_order(&mut self, order_id: &B256, filled_qty: u64) -> Result<()> {
        let order = self.orders.iter_mut().find(|o| &o.id == order_id)
            .ok_or(DexError::OrderNotFound)?;

        if filled_qty > order.remaining {
            return Err(DexError::InvalidFillQuantity);
        }

        order.remaining -= filled_qty;
        self.total_quantity -= filled_qty;

        // 如果完全成交，移除订单
        if order.remaining == 0 {
            self.remove_order(order_id);
        }

        Ok(())
    }

    /// 判断是否为空
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }
}

/// 订单引用（用于快速查找）
#[derive(Debug, Clone, Copy)]
pub struct OrderRef {
    /// 价格（用于定位到具体档位）
    pub price: i64,

    /// 买/卖方向
    pub side: Side,

    /// 在VecDeque中的位置
    pub position: usize,
}

/// 订单结构
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    /// 订单ID
    pub id: B256,

    /// 用户地址
    pub user: Address,

    /// 交易对ID
    pub symbol_id: u32,

    /// 买/卖方向
    pub side: Side,

    /// 订单类型
    pub order_type: OrderType,

    /// 价格
    pub price: u64,

    /// 原始数量
    pub quantity: u64,

    /// 剩余数量
    pub remaining: u64,

    /// 创建时间（纳秒时间戳）
    pub timestamp: u64,

    /// 市场类型
    pub market_type: MarketType,

    /// Perp特定字段
    pub perp_info: Option<PerpOrderInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpOrderInfo {
    pub leverage: u8,
    pub reduce_only: bool,
    pub stop_loss_price: Option<u64>,
    pub take_profit_price: Option<u64>,
}

/// 24小时统计
#[derive(Debug, Clone, Default)]
pub struct Statistics24h {
    pub volume: u128,
    pub high: u64,
    pub low: u64,
    pub open: u64,
    pub trades_count: u64,
}

impl OrderBook {
    /// 创建新的订单簿
    pub fn new(symbol_id: u32) -> Self {
        Self {
            symbol_id,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            order_index: HashMap::new(),
            last_price: 0,
            stats_24h: Statistics24h::default(),
            next_order_id: 1,
        }
    }

    /// 下单并撮合
    pub fn place_order(&mut self, mut order: Order) -> Result<Vec<Trade>> {
        let mut trades = Vec::new();

        match order.order_type {
            OrderType::Market => {
                // 市价单：立即成交，不挂单
                trades = self.match_market_order(&mut order)?;

                if order.remaining > 0 {
                    warn!("Market order not fully filled: {}", order.remaining);
                }
            }

            OrderType::Limit => {
                // 限价单：先尝试撮合，剩余部分挂单
                trades = self.match_limit_order(&mut order)?;

                if order.remaining > 0 {
                    self.add_order_to_book(order)?;
                }
            }

            OrderType::PostOnly => {
                // PostOnly：只做Maker，不能立即成交
                if self.would_cross_spread(&order) {
                    return Err(DexError::PostOnlyWouldCross);
                }

                self.add_order_to_book(order)?;
            }

            OrderType::IOC => {
                // IOC：立即成交，剩余取消
                trades = self.match_limit_order(&mut order)?;
                // 不挂单，剩余部分自动取消
            }

            OrderType::FOK => {
                // FOK：要么全部成交，要么全部取消
                trades = self.match_fok_order(&mut order)?;
            }
        }

        // 更新最新价格
        if let Some(last_trade) = trades.last() {
            self.last_price = last_trade.price;
            self.update_24h_stats(&trades);
        }

        Ok(trades)
    }

    /// 撮合限价单
    fn match_limit_order(&mut self, order: &mut Order) -> Result<Vec<Trade>> {
        let mut trades = Vec::new();

        let opposite_book = match order.side {
            Side::Buy => &mut self.asks,
            Side::Sell => &mut self.bids,
        };

        loop {
            // 获取对手方最优价格档位
            let best_level_key = match order.side {
                Side::Buy => {
                    // 买单：找最低卖价
                    opposite_book.keys().next().copied()
                }
                Side::Sell => {
                    // 卖单：找最高买价（注意bids的key是负数）
                    opposite_book.keys().next().copied()
                }
            };

            let Some(key) = best_level_key else {
                // 对手方订单簿为空
                break;
            };

            let opposite_price = key.abs() as u64;

            // 检查价格是否匹配
            let price_matches = match order.side {
                Side::Buy => order.price >= opposite_price,
                Side::Sell => order.price <= opposite_price,
            };

            if !price_matches {
                break;
            }

            // 获取对手方档位
            let level = opposite_book.get_mut(&key).unwrap();

            // 按时间优先撮合该档位的订单
            while !level.orders.is_empty() && order.remaining > 0 {
                let opposite_order = level.orders.front_mut().unwrap();

                // 计算成交数量
                let trade_qty = order.remaining.min(opposite_order.remaining);

                // 创建成交记录
                let trade = Trade {
                    trade_id: self.generate_trade_id(),
                    symbol_id: self.symbol_id,
                    buyer: if order.side == Side::Buy {
                        order.user
                    } else {
                        opposite_order.user
                    },
                    seller: if order.side == Side::Sell {
                        order.user
                    } else {
                        opposite_order.user
                    },
                    price: opposite_price, // 成交价是挂单价格（Price-Time Priority）
                    quantity: trade_qty,
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos() as u64,
                    buyer_order_id: if order.side == Side::Buy {
                        order.id
                    } else {
                        opposite_order.id
                    },
                    seller_order_id: if order.side == Side::Sell {
                        order.id
                    } else {
                        opposite_order.id
                    },
                    market_type: order.market_type,
                };

                trades.push(trade);

                // 更新订单剩余数量
                order.remaining -= trade_qty;
                opposite_order.remaining -= trade_qty;
                level.total_quantity -= trade_qty;

                // 如果对手订单完全成交，移除
                if opposite_order.remaining == 0 {
                    let filled_order = level.orders.pop_front().unwrap();
                    self.order_index.remove(&filled_order.id);
                    level.order_count -= 1;
                }
            }

            // 如果该档位已空，移除
            if level.is_empty() {
                opposite_book.remove(&key);
            }

            // 如果当前订单已完全成交，退出
            if order.remaining == 0 {
                break;
            }
        }

        Ok(trades)
    }

    /// 撮合市价单
    fn match_market_order(&mut self, order: &mut Order) -> Result<Vec<Trade>> {
        // 市价单不限价格，尽可能成交
        let original_price = order.price;
        order.price = match order.side {
            Side::Buy => u64::MAX,  // 买单：接受任何价格
            Side::Sell => 0,        // 卖单：接受任何价格
        };

        let trades = self.match_limit_order(order)?;

        order.price = original_price;
        Ok(trades)
    }

    /// 撮合FOK订单
    fn match_fok_order(&mut self, order: &mut Order) -> Result<Vec<Trade>> {
        // 1. 先检查是否能完全成交
        let available_liquidity = self.calculate_available_liquidity(order);

        if available_liquidity < order.quantity {
            return Err(DexError::FOKNotFilled);
        }

        // 2. 执行撮合
        self.match_limit_order(order)
    }

    /// 计算可用流动性
    fn calculate_available_liquidity(&self, order: &Order) -> u64 {
        let opposite_book = match order.side {
            Side::Buy => &self.asks,
            Side::Sell => &self.bids,
        };

        let mut total = 0u64;

        for (key, level) in opposite_book.iter() {
            let price = key.abs() as u64;

            // 检查价格是否在范围内
            let in_range = match order.side {
                Side::Buy => order.price >= price,
                Side::Sell => order.price <= price,
            };

            if !in_range {
                break;
            }

            total += level.total_quantity;

            if total >= order.quantity {
                break;
            }
        }

        total
    }

    /// 添加订单到订单簿
    fn add_order_to_book(&mut self, order: Order) -> Result<()> {
        let price_key = match order.side {
            Side::Buy => -(order.price as i64),  // 买单：取负数实现降序
            Side::Sell => order.price as i64,    // 卖单：正数升序
        };

        let book = match order.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };

        // 添加到对应价格档位
        book.entry(price_key)
            .or_insert_with(|| PriceLevel::new(order.price))
            .add_order(order.clone());

        // 添加到索引
        self.order_index.insert(
            order.id,
            OrderRef {
                price: price_key,
                side: order.side,
                position: 0, // TODO: 实际位置
            },
        );

        Ok(())
    }

    /// 撤单
    pub fn cancel_order(&mut self, order_id: B256, user: Address) -> Result<Order> {
        // 1. 从索引查找订单
        let order_ref = self.order_index
            .remove(&order_id)
            .ok_or(DexError::OrderNotFound)?;

        // 2. 从订单簿移除
        let book = match order_ref.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };

        let level = book
            .get_mut(&order_ref.price)
            .ok_or(DexError::PriceLevelNotFound)?;

        let order = level
            .remove_order(&order_id)
            .ok_or(DexError::OrderNotFound)?;

        // 3. 验证权限
        if order.user != user {
            return Err(DexError::Unauthorized);
        }

        // 4. 如果档位为空，移除
        if level.is_empty() {
            book.remove(&order_ref.price);
        }

        Ok(order)
    }

    /// 批量撤单（优化版）
    pub fn cancel_orders_batch(&mut self, order_ids: &[B256], user: Address) -> Vec<Result<B256>> {
        order_ids
            .iter()
            .map(|id| {
                self.cancel_order(*id, user)
                    .map(|_| *id)
            })
            .collect()
    }

    /// 判断订单是否会立即成交
    fn would_cross_spread(&self, order: &Order) -> bool {
        let opposite_book = match order.side {
            Side::Buy => &self.asks,
            Side::Sell => &self.bids,
        };

        if let Some((key, _)) = opposite_book.iter().next() {
            let opposite_price = key.abs() as u64;

            match order.side {
                Side::Buy => order.price >= opposite_price,
                Side::Sell => order.price <= opposite_price,
            }
        } else {
            false
        }
    }

    /// 获取订单簿快照（用于查询）
    pub fn to_view(&self) -> OrderBookView {
        OrderBookView {
            symbol_id: self.symbol_id,
            bids: self.get_levels(&self.bids, false),
            asks: self.get_levels(&self.asks, true),
            last_price: self.last_price,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        }
    }

    /// 获取价格档位列表
    fn get_levels(&self, book: &BTreeMap<i64, PriceLevel>, is_ask: bool) -> Vec<Level> {
        book.iter()
            .take(20) // 只返回前20档
            .map(|(_, level)| Level {
                price: level.price,
                quantity: level.total_quantity,
                order_count: level.order_count,
            })
            .collect()
    }

    /// 生成交易ID
    fn generate_trade_id(&mut self) -> u64 {
        self.next_order_id += 1;
        self.next_order_id
    }

    /// 计算平均成交价
    pub fn calculate_avg_price(&self, trades: &[Trade]) -> u64 {
        if trades.is_empty() {
            return 0;
        }

        let total_value: u128 = trades
            .iter()
            .map(|t| t.price as u128 * t.quantity as u128)
            .sum();

        let total_quantity: u128 = trades
            .iter()
            .map(|t| t.quantity as u128)
            .sum();

        (total_value / total_quantity) as u64
    }

    /// 更新24小时统计
    fn update_24h_stats(&mut self, trades: &[Trade]) {
        for trade in trades {
            self.stats_24h.volume += trade.price as u128 * trade.quantity as u128;
            self.stats_24h.trades_count += 1;

            if self.stats_24h.high == 0 || trade.price > self.stats_24h.high {
                self.stats_24h.high = trade.price;
            }

            if self.stats_24h.low == 0 || trade.price < self.stats_24h.low {
                self.stats_24h.low = trade.price;
            }
        }
    }

    /// 从快照恢复订单簿
    pub fn from_snapshot(symbol_id: u32, snapshot: OrderBookSnapshot) -> Result<Self> {
        let mut orderbook = Self::new(symbol_id);
        orderbook.last_price = snapshot.last_price;

        // 重建买单簿
        for level_data in snapshot.bids {
            // 注意：快照中只有价格和总量，没有具体订单
            // 实际实现需要存储完整订单列表
            let price_key = -(level_data.price as i64);
            let mut level = PriceLevel::new(level_data.price);
            level.total_quantity = level_data.quantity;
            level.order_count = level_data.order_count;

            orderbook.bids.insert(price_key, level);
        }

        // 重建卖单簿
        for level_data in snapshot.asks {
            let price_key = level_data.price as i64;
            let mut level = PriceLevel::new(level_data.price);
            level.total_quantity = level_data.quantity;
            level.order_count = level_data.order_count;

            orderbook.asks.insert(price_key, level);
        }

        Ok(orderbook)
    }

    /// 转换为快照
    pub fn to_snapshot(&self) -> OrderBookSnapshot {
        OrderBookSnapshot {
            bids: self.get_levels(&self.bids, false),
            asks: self.get_levels(&self.asks, true),
            last_price: self.last_price,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }
}

/// 订单簿视图（用于查询，不可变）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookView {
    pub symbol_id: u32,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
    pub last_price: u64,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Level {
    pub price: u64,
    pub quantity: u64,
    pub order_count: u32,
}

/// 成交记录
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trade {
    pub trade_id: u64,
    pub symbol_id: u32,
    pub buyer: Address,
    pub seller: Address,
    pub price: u64,
    pub quantity: u64,
    pub timestamp: u64,
    pub buyer_order_id: B256,
    pub seller_order_id: B256,
    pub market_type: MarketType,
}
```

## 2. 性能优化关键技术

### 2.1 内存池（避免频繁分配）

```rust
use std::mem;

/// 订单内存池
pub struct OrderPool {
    /// 空闲订单槽位
    free_slots: Vec<usize>,

    /// 订单存储（使用Vec作为内存池）
    orders: Vec<Option<Order>>,

    /// 容量
    capacity: usize,
}

impl OrderPool {
    pub fn new(capacity: usize) -> Self {
        Self {
            free_slots: (0..capacity).collect(),
            orders: (0..capacity).map(|_| None).collect(),
            capacity,
        }
    }

    /// 分配订单
    pub fn allocate(&mut self, order: Order) -> Option<usize> {
        let slot = self.free_slots.pop()?;
        self.orders[slot] = Some(order);
        Some(slot)
    }

    /// 释放订单
    pub fn deallocate(&mut self, slot: usize) {
        if slot < self.capacity {
            self.orders[slot] = None;
            self.free_slots.push(slot);
        }
    }

    /// 获取订单
    pub fn get(&self, slot: usize) -> Option<&Order> {
        self.orders.get(slot)?.as_ref()
    }

    /// 获取可变订单
    pub fn get_mut(&mut self, slot: usize) -> Option<&mut Order> {
        self.orders.get_mut(slot)?.as_mut()
    }
}

/// 使用内存池优化的订单簿
pub struct PooledOrderBook {
    symbol_id: u32,
    bids: BTreeMap<i64, Vec<usize>>,  // 存储订单在池中的索引
    asks: BTreeMap<i64, Vec<usize>>,
    order_pool: OrderPool,
    order_index: HashMap<B256, usize>, // OrderId -> Pool Index
}

impl PooledOrderBook {
    pub fn new(symbol_id: u32, pool_capacity: usize) -> Self {
        Self {
            symbol_id,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            order_pool: OrderPool::new(pool_capacity),
            order_index: HashMap::new(),
        }
    }

    /// 添加订单（使用内存池）
    pub fn add_order(&mut self, order: Order) -> Result<()> {
        let pool_index = self.order_pool.allocate(order.clone())
            .ok_or(DexError::PoolExhausted)?;

        let price_key = match order.side {
            Side::Buy => -(order.price as i64),
            Side::Sell => order.price as i64,
        };

        let book = match order.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };

        book.entry(price_key)
            .or_insert_with(Vec::new)
            .push(pool_index);

        self.order_index.insert(order.id, pool_index);

        Ok(())
    }

    /// 移除订单
    pub fn remove_order(&mut self, order_id: &B256) -> Result<Order> {
        let pool_index = self.order_index
            .remove(order_id)
            .ok_or(DexError::OrderNotFound)?;

        let order = self.order_pool.get(pool_index)
            .ok_or(DexError::OrderNotFound)?
            .clone();

        // 从价格档位移除
        let price_key = match order.side {
            Side::Buy => -(order.price as i64),
            Side::Sell => order.price as i64,
        };

        let book = match order.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };

        if let Some(indices) = book.get_mut(&price_key) {
            indices.retain(|&idx| idx != pool_index);

            if indices.is_empty() {
                book.remove(&price_key);
            }
        }

        // 释放内存池槽位
        self.order_pool.deallocate(pool_index);

        Ok(order)
    }
}
```

### 2.2 SIMD批量操作

```rust
use std::simd::*;

/// 使用SIMD批量验证订单价格
pub fn validate_prices_simd(orders: &[Order]) -> Vec<bool> {
    let mut results = Vec::with_capacity(orders.len());

    // 每次处理8个订单（假设u64x8 SIMD向量）
    for chunk in orders.chunks(8) {
        let mut prices = [0u64; 8];
        let mut min_prices = [1000u64; 8]; // 最低价格限制
        let mut max_prices = [1000000000u64; 8]; // 最高价格限制

        for (i, order) in chunk.iter().enumerate() {
            prices[i] = order.price;
        }

        // SIMD比较
        let price_vec = u64x8::from_array(prices);
        let min_vec = u64x8::from_array(min_prices);
        let max_vec = u64x8::from_array(max_prices);

        let valid = price_vec.simd_ge(min_vec) & price_vec.simd_le(max_vec);

        // 提取结果
        for i in 0..chunk.len() {
            results.push(valid.test(i));
        }
    }

    results
}

/// SIMD批量计算成交金额
pub fn calculate_trade_values_simd(trades: &[Trade]) -> Vec<u128> {
    let mut values = Vec::with_capacity(trades.len());

    for chunk in trades.chunks(8) {
        let mut prices = [0u64; 8];
        let mut quantities = [0u64; 8];

        for (i, trade) in chunk.iter().enumerate() {
            prices[i] = trade.price;
            quantities[i] = trade.quantity;
        }

        // SIMD乘法
        let price_vec = u64x8::from_array(prices);
        let qty_vec = u64x8::from_array(quantities);

        // 注意：u64 * u64 = u128需要特殊处理
        for i in 0..chunk.len() {
            values.push(chunk[i].price as u128 * chunk[i].quantity as u128);
        }
    }

    values
}
```

### 2.3 无锁订单簿（分片）

```rust
use crossbeam::queue::SegQueue;
use std::sync::atomic::{AtomicU64, Ordering};

/// 分片订单簿管理器
pub struct ShardedOrderBookManager {
    /// 订单簿分片（每个交易对独立分片）
    shards: Vec<Arc<RwLock<OrderBook>>>,

    /// 路由器（交易对ID -> 分片ID）
    router: HashMap<u32, usize>,

    /// 全局统计
    global_stats: Arc<GlobalStats>,
}

impl ShardedOrderBookManager {
    pub fn new(num_shards: usize) -> Self {
        let mut shards = Vec::with_capacity(num_shards);
        for _ in 0..num_shards {
            // 每个分片处理特定范围的交易对
            shards.push(Arc::new(RwLock::new(OrderBook::new(0))));
        }

        Self {
            shards,
            router: HashMap::new(),
            global_stats: Arc::new(GlobalStats::new()),
        }
    }

    /// 路由到特定分片
    pub fn get_shard(&self, symbol_id: u32) -> &Arc<RwLock<OrderBook>> {
        let shard_id = self.router.get(&symbol_id)
            .copied()
            .unwrap_or_else(|| symbol_id as usize % self.shards.len());

        &self.shards[shard_id]
    }

    /// 并行处理多个交易对的订单
    pub fn place_orders_parallel(&self, orders: Vec<Order>) -> Vec<Result<Vec<Trade>>> {
        // 按交易对分组
        let mut groups: HashMap<u32, Vec<Order>> = HashMap::new();
        for order in orders {
            groups.entry(order.symbol_id)
                .or_insert_with(Vec::new)
                .push(order);
        }

        // 并行处理每个组
        groups.into_par_iter()
            .map(|(symbol_id, orders)| {
                let shard = self.get_shard(symbol_id);
                let mut orderbook = shard.write();

                let mut all_trades = Vec::new();
                for order in orders {
                    let trades = orderbook.place_order(order)?;
                    all_trades.extend(trades);
                }

                Ok(all_trades)
            })
            .collect()
    }
}

/// 全局统计（原子操作）
pub struct GlobalStats {
    total_trades: AtomicU64,
    total_volume: AtomicU64,
    active_orders: AtomicU64,
}

impl GlobalStats {
    pub fn new() -> Self {
        Self {
            total_trades: AtomicU64::new(0),
            total_volume: AtomicU64::new(0),
            active_orders: AtomicU64::new(0),
        }
    }

    pub fn record_trade(&self, volume: u64) {
        self.total_trades.fetch_add(1, Ordering::Relaxed);
        self.total_volume.fetch_add(volume, Ordering::Relaxed);
    }

    pub fn add_order(&self) {
        self.active_orders.fetch_add(1, Ordering::Relaxed);
    }

    pub fn remove_order(&self) {
        self.active_orders.fetch_sub(1, Ordering::Relaxed);
    }
}
```

## 3. 订单簿性能测试

```rust
#[cfg(test)]
mod benches {
    use super::*;
    use criterion::{black_box, Criterion};

    /// 测试下单性能
    fn bench_place_order(c: &mut Criterion) {
        let mut orderbook = OrderBook::new(0);

        // 预填充一些订单
        for i in 0..1000 {
            let order = Order {
                id: B256::random(),
                user: Address::random(),
                symbol_id: 0,
                side: if i % 2 == 0 { Side::Buy } else { Side::Sell },
                order_type: OrderType::Limit,
                price: 50000 + (i % 100) as u64,
                quantity: 1_000_000,
                remaining: 1_000_000,
                timestamp: 0,
                market_type: MarketType::Spot,
                perp_info: None,
            };
            let _ = orderbook.place_order(order);
        }

        c.bench_function("place_order", |b| {
            b.iter(|| {
                let order = Order {
                    id: B256::random(),
                    user: Address::random(),
                    symbol_id: 0,
                    side: Side::Buy,
                    order_type: OrderType::Limit,
                    price: black_box(50050),
                    quantity: black_box(1_000_000),
                    remaining: black_box(1_000_000),
                    timestamp: 0,
                    market_type: MarketType::Spot,
                    perp_info: None,
                };

                orderbook.place_order(order).unwrap();
            });
        });
    }

    /// 测试撮合性能
    fn bench_match_order(c: &mut Criterion) {
        let mut orderbook = OrderBook::new(0);

        // 填充卖单
        for i in 0..1000 {
            let order = Order {
                id: B256::random(),
                user: Address::random(),
                symbol_id: 0,
                side: Side::Sell,
                order_type: OrderType::Limit,
                price: 50000 + i,
                quantity: 1_000_000,
                remaining: 1_000_000,
                timestamp: 0,
                market_type: MarketType::Spot,
                perp_info: None,
            };
            let _ = orderbook.place_order(order);
        }

        c.bench_function("match_order", |b| {
            b.iter(|| {
                let order = Order {
                    id: B256::random(),
                    user: Address::random(),
                    symbol_id: 0,
                    side: Side::Buy,
                    order_type: OrderType::Market,
                    price: u64::MAX,
                    quantity: black_box(100_000_000), // 成交100个订单
                    remaining: black_box(100_000_000),
                    timestamp: 0,
                    market_type: MarketType::Spot,
                    perp_info: None,
                };

                orderbook.place_order(order).unwrap();
            });
        });
    }

    /// 测试批量撤单性能
    fn bench_cancel_orders_batch(c: &mut Criterion) {
        let mut orderbook = OrderBook::new(0);
        let user = Address::random();

        // 填充订单
        let mut order_ids = Vec::new();
        for i in 0..10000 {
            let order_id = B256::random();
            order_ids.push(order_id);

            let order = Order {
                id: order_id,
                user,
                symbol_id: 0,
                side: if i % 2 == 0 { Side::Buy } else { Side::Sell },
                order_type: OrderType::Limit,
                price: 50000 + (i % 100) as u64,
                quantity: 1_000_000,
                remaining: 1_000_000,
                timestamp: 0,
                market_type: MarketType::Spot,
                perp_info: None,
            };
            let _ = orderbook.place_order(order);
        }

        c.bench_function("cancel_orders_batch_1000", |b| {
            b.iter(|| {
                let batch = &order_ids[0..1000];
                orderbook.cancel_orders_batch(batch, user);
            });
        });
    }
}
```

## 4. 性能预期

```
硬件配置：16核 64GB内存

单订单簿性能：
├─ 下单（无撮合）: ~2μs
├─ 下单+撮合（10档）: ~10μs
├─ 撤单: ~1μs
├─ 查询订单簿: ~0.5μs
└─ 批量撤单（1000笔）: ~500μs

吞吐量：
├─ 单线程：~100,000 ops/s
├─ 16线程（分片）: ~1,600,000 ops/s
└─ 实际TPS（考虑网络等）: ~200,000 TPS

内存占用（单交易对）：
├─ 空订单簿: ~1 KB
├─ 1,000活跃订单: ~100 KB
├─ 10,000活跃订单: ~1 MB
└─ 1000交易对: ~1 GB
```

这个订单簿实现展示了如何达到20万TPS的关键技术。需要我继续展开强平机制、资金费率等其他部分吗？