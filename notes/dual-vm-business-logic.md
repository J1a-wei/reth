# 双VM架构：Spot + Perp 业务逻辑划分与状态管理

## 1. 业务视角：双VM如何协同工作

### 1.1 核心原则

```
EVM：负责"决策逻辑" - 复杂交易策略、风控规则、用户配置
DEXVM：负责"执行动作" - 订单撮合、持仓管理、实时清算

类比：
EVM = 大脑（策略、判断、配置）
DEXVM = 交易引擎（撮合、结算、风控执行）

业务聚焦：
├─ Spot（现货交易）：即时交割的币币交易
└─ Perp（永续合约）：带杠杆的衍生品交易
```

### 1.2 实际业务流程示例

#### 场景1：Spot现货交易（纯DEXVM，最快路径）

```
用户: BTC/USDT现货市场，买入1个BTC，限价50000 USDT

流程:
1. 用户签名DEXVM交易
   ↓
2. 直接进入DEXVM（不经过EVM）
   ├─ 验证USDT余额 >= 50000
   ├─ 下单到BTC/USDT订单簿
   ├─ 立即撮合可成交部分
   │  └─ 买入0.8 BTC @ 49900 USDT (成交)
   ├─ 未成交0.2 BTC挂单在订单簿
   └─ 更新余额：
       USDT: -39920 (0.8 * 49900)
       BTC:  +0.8
   ↓
3. 返回结果：已成交0.8 BTC，均价49900，挂单0.2 BTC

关键：全程不经过EVM，纯DEXVM执行
性能：~75μs延迟，支持20万TPS
```

**代码实现**：
```rust
// 用户签名的DEXVM交易
pub struct DexTransaction {
    from: Address,
    dex_bytecode: Vec<u8>,  // DEXVM字节码
    signature: Signature,
    gas_limit: u64,
}

// DEXVM字节码（汇编级）
// PUSH BTC/USD
// PUSH 0 (Buy)
// PUSH 50000 (price)
// PUSH 1.0 (quantity)
// PLACE_LIMIT
// RETURN

// 执行路径：
// Transaction Pool → DEXVM → Matching Engine → State Update
// 不经过EVM，速度极快
```

#### 场景2：Perp永续合约交易（纯DEXVM，高频路径）

```
用户: BTC-PERP市场，10x杠杆做多1 BTC，价格50000 USDT

流程:
1. 用户签名DEXVM交易
   ↓
2. 直接进入DEXVM
   ├─ 验证保证金：需要5000 USDT (50000 / 10)
   ├─ 下单到BTC-PERP订单簿
   ├─ 立即撮合
   │  └─ 开仓1 BTC @ 50000 USDT
   ├─ 更新持仓状态：
   │  Position: +1 BTC (多头)
   │  Entry Price: 50000
   │  Margin: 5000 USDT (已冻结)
   │  Leverage: 10x
   └─ 计算强平价格: 45000 USDT
   ↓
3. 返回结果：开仓成功，持仓+1 BTC，强平价45000

关键：全程不经过EVM，纯DEXVM执行
性能：~75μs延迟，支持20万TPS
```

#### 场景3：网格交易策略（EVM + DEXVM协同）

```
场景: Spot网格交易机器人（Solidity编写）

流程:
1. 用户调用智能合约 startSpotGrid()
   ↓
2. EVM执行策略逻辑
   ├─ 查询BTC/USDT当前价格（调用预编译查DEXVM）
   ├─ 计算网格价格（EVM中的复杂计算）
   │  Base: 50000, Grid: 500, Levels: 20
   │  Buy:  [49500, 49000, 48500, ...]
   │  Sell: [50500, 51000, 51500, ...]
   ├─ 验证用户余额（10万USDT是否足够）
   ├─ 构造40个订单（20买 + 20卖）
   └─ 调用预编译 batchPlaceOrders()
   ↓
3. 预编译转发到DEXVM
   ├─ 批量下单到BTC/USDT订单簿
   ├─ 立即撮合部分订单
   ├─ 其余挂单在订单簿等待
   └─ 返回40个OrderResult
   ↓
4. EVM记录订单ID（方便后续管理）
   └─ 触发GridStarted事件

关键：策略决策在EVM，订单执行在DEXVM
性能：EVM部分~500μs，DEXVM部分~100μs，总计~600μs
```

**代码实现**：
```solidity
contract GridTradingBot {
    using DexPrecompiles for *;

    // 业务状态（存在EVM）
    struct GridConfig {
        bytes32 symbol;
        uint128 basePrice;
        uint128 gridSize;
        uint8 gridLevels;
        bool active;
    }

    mapping(address => GridConfig) public userGrids;
    mapping(address => bytes32[]) public activeOrders; // 记录订单ID

    /// 用户触发网格交易
    function startGrid(
        bytes32 symbol,
        uint128 basePrice,
        uint128 gridSize,
        uint8 gridLevels
    ) external {
        // === 在EVM中执行的业务逻辑 ===

        // 1. 权限检查
        require(!userGrids[msg.sender].active, "Grid already active");

        // 2. 风险检查（复杂计算，适合EVM）
        uint256 totalMarginRequired = calculateMarginRequired(
            symbol, basePrice, gridSize, gridLevels
        );
        require(checkUserMargin(msg.sender, totalMarginRequired), "Insufficient margin");

        // 3. 构造订单（在EVM中准备数据）
        DexPrecompiles.Order[] memory orders = new DexPrecompiles.Order[](gridLevels * 2);

        for (uint8 i = 0; i < gridLevels; i++) {
            orders[i * 2] = DexPrecompiles.Order({
                symbol: symbol,
                side: DexPrecompiles.Side.Buy,
                orderType: DexPrecompiles.OrderType.Limit,
                price: basePrice - gridSize * (i + 1),
                quantity: 1e18,
                leverage: 1,
                reduceOnly: false
            });

            orders[i * 2 + 1] = DexPrecompiles.Order({
                symbol: symbol,
                side: DexPrecompiles.Side.Sell,
                orderType: DexPrecompiles.OrderType.Limit,
                price: basePrice + gridSize * (i + 1),
                quantity: 1e18,
                leverage: 1,
                reduceOnly: false
            });
        }

        // === 调用DEXVM执行（高性能路径）===
        DexPrecompiles.OrderResult[] memory results =
            DexPrecompiles.batchPlaceOrders(orders);

        // === 回到EVM处理结果 ===

        // 4. 记录订单ID（方便后续管理）
        for (uint i = 0; i < results.length; i++) {
            activeOrders[msg.sender].push(results[i].orderId);
        }

        // 5. 保存配置
        userGrids[msg.sender] = GridConfig({
            symbol: symbol,
            basePrice: basePrice,
            gridSize: gridSize,
            gridLevels: gridLevels,
            active: true
        });

        // 6. 触发事件（用于前端监听）
        emit GridStarted(msg.sender, symbol, gridLevels);
    }

    /// 停止网格交易
    function stopGrid() external {
        require(userGrids[msg.sender].active, "No active grid");

        // === 批量撤单（调用DEXVM）===
        bytes32[] memory orderIds = activeOrders[msg.sender];
        DexPrecompiles.batchCancelOrders(orderIds);

        // === EVM清理状态 ===
        delete activeOrders[msg.sender];
        userGrids[msg.sender].active = false;

        emit GridStopped(msg.sender);
    }

    // === 辅助函数（在EVM中执行）===

    function calculateMarginRequired(
        bytes32 symbol,
        uint128 basePrice,
        uint128 gridSize,
        uint8 gridLevels
    ) internal pure returns (uint256) {
        // 复杂计算逻辑，适合EVM
        // 这里可以使用Solidity的丰富表达能力
    }

    function checkUserMargin(address user, uint256 required) internal view returns (bool) {
        // 查询用户保证金（跨EVM和DEXVM状态）
        // 实际实现会调用预编译查询
    }

    event GridStarted(address indexed user, bytes32 symbol, uint8 levels);
    event GridStopped(address indexed user);
}
```

#### 场景4：Perp自动止盈止损（EVM + DEXVM协同）

```
场景: 用户持有BTC-PERP多头仓位，设置止盈止损

初始状态:
- 持仓: +10 BTC @ 50000 (开仓价)
- 保证金: 50000 USDT (10x杠杆)
- 当前价格: 52000 USDT

流程:
1. 用户调用EVM合约 setStopLoss()
   ↓
2. EVM存储止盈止损配置
   ├─ Stop Loss: 48000 USDT (-4%)
   ├─ Take Profit: 55000 USDT (+10%)
   ├─ 验证价格合理性
   └─ 触发StopLossSet事件
   ↓
3. Keeper监控价格变化
   ├─ 定期查询DEXVM最新价格
   └─ 检查是否触发条件
   ↓
4. 价格跌至47900（触发止损）
   ↓
5. Keeper调用EVM合约 executeStopLoss()
   ↓
6. EVM验证触发条件
   ├─ 查询DEXVM当前价格
   ├─ 确认 <= 止损价
   └─ 调用预编译平仓
   ↓
7. DEXVM执行市价平仓
   ├─ 卖出10 BTC @ ~48000
   ├─ 结算盈亏: -20000 USDT (亏损)
   ├─ 释放保证金: 30000 USDT
   └─ 清空持仓
   ↓
8. EVM清理止损配置
   └─ 触发StopLossExecuted事件

关键：
- 配置存储在EVM（灵活修改）
- 触发判断在EVM（复杂逻辑）
- 平仓执行在DEXVM（高性能）
```

**代码实现**：
```solidity
contract PerpStopLoss {
    using DexPrecompiles for *;

    bytes32 public constant BTC_PERP = keccak256("BTC-PERP");
    bytes32 public constant ETH_PERP = keccak256("ETH-PERP");

    struct StopLossConfig {
        bytes32 symbol;          // 交易对
        uint128 stopLossPrice;   // 止损价
        uint128 takeProfitPrice; // 止盈价
        uint128 entryPrice;      // 开仓价
        uint128 positionSize;    // 持仓数量
        bool active;
    }

    mapping(address => StopLossConfig) public stopLossConfigs;

    /// 设置止盈止损
    function setStopLoss(
        bytes32 symbol,
        uint128 stopLossPrice,
        uint128 takeProfitPrice
    ) external {
        // === EVM: 验证和配置 ===

        // 1. 查询用户当前持仓（DEXVM）
        DexPrecompiles.Position memory pos =
            DexPrecompiles.queryPosition(msg.sender, symbol);
        require(pos.size > 0, "No position");

        // 2. 验证价格合理性（EVM中的复杂逻辑）
        if (pos.size > 0) {
            // 多头持仓
            require(stopLossPrice < pos.entryPrice, "Stop loss must < entry");
            require(takeProfitPrice > pos.entryPrice, "Take profit must > entry");
            require(
                (pos.entryPrice - stopLossPrice) * 100 / pos.entryPrice <= 20,
                "Stop loss > 20% not allowed"
            );
        } else {
            // 空头持仓
            require(stopLossPrice > pos.entryPrice, "Stop loss must > entry");
            require(takeProfitPrice < pos.entryPrice, "Take profit must < entry");
        }

        // 3. 保存配置（EVM状态）
        stopLossConfigs[msg.sender] = StopLossConfig({
            symbol: symbol,
            stopLossPrice: stopLossPrice,
            takeProfitPrice: takeProfitPrice,
            entryPrice: pos.entryPrice,
            positionSize: pos.size,
            active: true
        });

        emit StopLossSet(msg.sender, symbol, stopLossPrice, takeProfitPrice);
    }

    /// 检查并执行止盈止损（Keeper调用）
    function checkAndExecute(address user) external {
        StopLossConfig memory config = stopLossConfigs[user];
        require(config.active, "No active config");

        // === EVM: 查询和判断 ===

        // 1. 查询当前价格（DEXVM）
        DexPrecompiles.OrderBook memory book =
            DexPrecompiles.queryOrderBook(config.symbol, 1);
        uint128 currentPrice = book.lastPrice;

        // 2. 判断是否触发（EVM逻辑）
        bool triggerStopLoss = false;
        bool triggerTakeProfit = false;

        if (config.positionSize > 0) {
            // 多头
            triggerStopLoss = currentPrice <= config.stopLossPrice;
            triggerTakeProfit = currentPrice >= config.takeProfitPrice;
        } else {
            // 空头
            triggerStopLoss = currentPrice >= config.stopLossPrice;
            triggerTakeProfit = currentPrice <= config.takeProfitPrice;
        }

        if (!triggerStopLoss && !triggerTakeProfit) {
            revert("No trigger condition met");
        }

        // === DEXVM: 执行平仓 ===

        // 3. 市价平仓
        DexPrecompiles.Order memory order = DexPrecompiles.Order({
            symbol: config.symbol,
            side: config.positionSize > 0
                ? DexPrecompiles.Side.Sell
                : DexPrecompiles.Side.Buy,
            orderType: DexPrecompiles.OrderType.Market,
            price: 0,
            quantity: uint128(abs(config.positionSize)),
            leverage: 1,
            reduceOnly: true // 只平仓
        });

        DexPrecompiles.OrderResult memory result =
            DexPrecompiles.placeOrder(order);

        // === EVM: 清理配置 ===

        // 4. 计算盈亏
        int256 pnl = triggerStopLoss
            ? int256(uint256(config.stopLossPrice)) - int256(uint256(config.entryPrice))
            : int256(uint256(config.takeProfitPrice)) - int256(uint256(config.entryPrice));

        pnl = pnl * int256(uint256(config.positionSize)) / 1e8;

        delete stopLossConfigs[user];

        emit StopLossExecuted(
            user,
            config.symbol,
            result.avgPrice,
            pnl,
            triggerStopLoss
        );
    }

    /// 手动取消止盈止损
    function cancelStopLoss() external {
        require(stopLossConfigs[msg.sender].active, "No active config");
        delete stopLossConfigs[msg.sender];
        emit StopLossCancelled(msg.sender);
    }

    function abs(int128 x) internal pure returns (uint128) {
        return x >= 0 ? uint128(x) : uint128(-x);
    }

    event StopLossSet(
        address indexed user,
        bytes32 symbol,
        uint128 stopLoss,
        uint128 takeProfit
    );
    event StopLossExecuted(
        address indexed user,
        bytes32 symbol,
        uint128 exitPrice,
        int256 pnl,
        bool isStopLoss
    );
    event StopLossCancelled(address indexed user);
}
```

## 2. 业务逻辑划分准则

### 2.1 归集到EVM的业务逻辑

| 业务类型 | 原因 | Spot示例 | Perp示例 |
|---------|------|---------|---------|
| **交易策略** | 需要复杂决策逻辑 | - 网格交易<br>- 价差套利<br>- DCA定投 | - 网格合约<br>- 动态对冲<br>- 趋势跟踪 |
| **风控配置** | 需要灵活的规则设置 | - 交易限额<br>- 价格偏离保护<br>- 滑点控制 | - 止盈止损<br>- 最大杠杆限制<br>- 持仓上限 |
| **用户配置** | 需要永久存储偏好 | - 默认交易对<br>- 手续费折扣<br>- 通知设置 | - 默认杠杆<br>- 保证金模式<br>- 自动追加保证金 |
| **权限管理** | 需要灵活的访问控制 | - API Key管理<br>- 子账户权限<br>- 白名单 | - 交易权限<br>- 提现权限<br>- 杠杆权限 |
| **条件触发** | 需要复杂的触发条件 | - 价格触发下单<br>- 批量撤单<br>- 冰山委托 | - 止盈止损<br>- 条件平仓<br>- 自动减仓 |
| **历史记录** | 需要链上可查询记录 | - 策略执行历史<br>- 配置变更记录 | - 强平历史<br>- 费率变更<br>- 保证金调整 |
| **事件通知** | 需要向外部系统发送事件 | - 成交通知<br>- 余额变动<br>- 异常告警 | - 强平预警<br>- 持仓变动<br>- 资金费率 |

**示例：止盈止损策略**
```solidity
contract StopLossTakeProfit {
    // 存储在EVM：策略配置
    struct Strategy {
        bytes32 dexOrderId;      // DEXVM中的持仓ID
        uint128 stopLossPrice;   // 止损价
        uint128 takeProfitPrice; // 止盈价
        bool active;
    }

    mapping(address => Strategy) public strategies;

    /// 设置止盈止损（EVM逻辑）
    function setStrategy(
        bytes32 orderId,
        uint128 stopLoss,
        uint128 takeProfit
    ) external {
        // 1. 验证持仓存在（查询DEXVM）
        DexPrecompiles.Position memory pos =
            DexPrecompiles.queryPosition(msg.sender, BTC_USD);
        require(pos.size > 0, "No position");

        // 2. 验证价格合理性（EVM中的复杂逻辑）
        require(stopLoss < pos.entryPrice, "Stop loss too high");
        require(takeProfit > pos.entryPrice, "Take profit too low");
        require(
            (pos.entryPrice - stopLoss) * 100 / pos.entryPrice <= 20,
            "Stop loss > 20%"
        );

        // 3. 保存策略（EVM状态）
        strategies[msg.sender] = Strategy({
            dexOrderId: orderId,
            stopLossPrice: stopLoss,
            takeProfitPrice: takeProfit,
            active: true
        });
    }

    /// 检查并触发（Keeper调用）
    function checkAndTrigger(address user) external {
        Strategy memory strat = strategies[user];
        require(strat.active, "No active strategy");

        // 1. 查询当前价格（DEXVM）
        DexPrecompiles.OrderBook memory book =
            DexPrecompiles.queryOrderBook(BTC_USD, 1);
        uint128 currentPrice = book.lastPrice;

        // 2. 判断是否触发（EVM逻辑）
        bool shouldTrigger =
            currentPrice <= strat.stopLossPrice ||
            currentPrice >= strat.takeProfitPrice;

        if (!shouldTrigger) return;

        // 3. 执行平仓（DEXVM）
        DexPrecompiles.Order memory order = DexPrecompiles.Order({
            symbol: BTC_USD,
            side: DexPrecompiles.Side.Sell,
            orderType: DexPrecompiles.OrderType.Market,
            price: 0,
            quantity: type(uint128).max, // 全部平仓
            leverage: 1,
            reduceOnly: true
        });

        DexPrecompiles.placeOrder(order);

        // 4. 清理策略（EVM状态）
        delete strategies[user];

        emit StrategyTriggered(user, currentPrice);
    }
}
```

### 2.2 归集到DEXVM的业务逻辑

| 业务类型 | 原因 | Spot示例 | Perp示例 |
|---------|------|---------|---------|
| **订单管理** | 核心交易功能，需要极致性能 | - 限价/市价下单<br>- 撤单/改单<br>- IOC/FOK订单 | - 开仓/加仓<br>- 平仓/减仓<br>- 只减仓订单 |
| **订单撮合** | 高频操作，专用算法 | - 价格匹配<br>- 成交执行<br>- 订单簿更新 | - 价格匹配<br>- 持仓更新<br>- 盈亏计算 |
| **余额管理** | 实时验证，高频访问 | - 余额检查<br>- 冻结/解冻<br>- 转账 | - 保证金检查<br>- 保证金冻结<br>- 盈亏结算 |
| **持仓管理** | 与撮合紧密耦合 | N/A（现货无持仓） | - 持仓更新<br>- 未实现盈亏<br>- 强平价格 |
| **风险检查** | 实时验证，必须快速 | - 余额充足性<br>- 价格偏离检查 | - 保证金充足性<br>- 杠杆限制<br>- 最大持仓 |
| **清算结算** | 批量操作，需要原子性 | - 成交结算<br>- 手续费扣除 | - 资金费率<br>- 强制平仓<br>- 自动减仓ADL |
| **市场数据** | 高频查询，需要实时 | - 订单簿深度<br>- 最新价格<br>- 24h成交量 | - 持仓分布<br>- 资金费率<br>- 标记价格 |

**示例：纯DEXVM交易流程**
```rust
// DEXVM字节码程序：高频交易策略
pub fn generate_hft_strategy_bytecode() -> Vec<u8> {
    let mut asm = DexAssembler::new();

    // ===== 高频交易策略（全在DEXVM中执行）=====

    // 1. 查询订单簿
    asm.push(symbol("BTC/USD"));
    asm.push(U256::from(10)); // depth = 10
    asm.opcode(DexOpcode::QueryOrderBook);
    // Stack: [orderbook_ptr]

    // 2. 分析盘口（在DEXVM中完成）
    asm.opcode(DexOpcode::AnalyzeSpread); // 自定义指令
    // Stack: [best_bid, best_ask, spread]

    // 3. 判断是否有套利机会
    asm.dup2(); // 复制 best_bid, best_ask
    asm.opcode(DexOpcode::Sub);
    asm.push(U256::from(5)); // spread < $5
    asm.opcode(DexOpcode::Lt);
    // Stack: [best_bid, best_ask, spread, is_tight]

    // 4. 如果spread太小，不交易
    asm.push(U256::from(100)); // jump to end
    asm.opcode(DexOpcode::JumpIf);

    // 5. 下单做市（买单）
    asm.push(symbol("BTC/USD"));
    asm.push(U256::ZERO); // Buy
    asm.swap(3); // 取出 best_bid
    asm.push(U256::from(1)); // 改进1美元
    asm.opcode(DexOpcode::Add);
    asm.push(U256::from(1e8 as u64)); // 0.01 BTC
    asm.push(U256::from(1)); // 1x leverage
    asm.opcode(DexOpcode::PlaceLimit);
    // Stack: [buy_order_id, best_ask, spread]

    // 6. 下单做市（卖单）
    asm.push(symbol("BTC/USD"));
    asm.push(U256::from(1)); // Sell
    asm.swap(2); // 取出 best_ask
    asm.push(U256::from(1)); // 降低1美元
    asm.opcode(DexOpcode::Sub);
    asm.push(U256::from(1e8 as u64)); // 0.01 BTC
    asm.push(U256::from(1));
    asm.opcode(DexOpcode::PlaceLimit);
    // Stack: [buy_order_id, sell_order_id]

    // 7. 返回
    asm.opcode(DexOpcode::Return);

    asm.bytecode()
}

// 关键：全程在DEXVM执行，无需EVM参与
// 延迟：~50μs
// 吞吐：20万TPS
```

### 2.3 决策树：我的业务应该放在哪里？

```
开始：我要实现一个DEX功能
  ↓
是否是交易执行（下单/撤单/平仓）？
  ├─ 是 → 是否需要复杂条件判断？
  │   ├─ 否 → 【DEXVM】直接执行
  │   │   示例：
  │   │   - 用户手动下单/撤单
  │   │   - API直接交易
  │   │   - 做市商策略（预编译的DEXVM字节码）
  │   │
  │   └─ 是 → 【EVM + 预编译】策略在EVM，执行在DEXVM
  │       示例：
  │       - 网格交易（需要计算多个价格）
  │       - 条件单（价格触发）
  │       - 止盈止损（持仓监控 + 平仓）
  │
  └─ 否 → 是否需要链上存储？
      ├─ 是 → 【EVM】配置和状态管理
      │   示例：
      │   - 用户配置（默认杠杆、保证金模式）
      │   - 策略参数（网格配置、止损价格）
      │   - 权限管理（子账户、API Key）
      │   - 历史记录（强平记录、费率调整）
      │
      └─ 否 → 【查询】只读操作
          示例：
          - 查询订单簿
          - 查询持仓
          - 查询成交历史

业务场景映射：

【纯DEXVM - 20万TPS】
├─ Spot现货
│  ├─ 限价/市价买卖
│  ├─ 撤单/改单
│  ├─ 高频做市
│  └─ 简单套利
│
└─ Perp永续
   ├─ 开仓/平仓/加减仓
   ├─ 保证金调整
   ├─ 强制平仓
   └─ 资金费率结算

【EVM + 预编译 - 10-15万TPS】
├─ Spot策略
│  ├─ 网格交易机器人
│  ├─ DCA定投策略
│  ├─ 价格触发下单
│  └─ 批量订单管理
│
└─ Perp策略
   ├─ 止盈止损自动化
   ├─ 动态对冲策略
   ├─ 仓位再平衡
   └─ 跟踪止损

【纯EVM - ~5k TPS】
├─ 用户配置
│  ├─ 交易偏好设置
│  ├─ 风控参数配置
│  └─ 通知设置
│
├─ 权限管理
│  ├─ 多签钱包
│  ├─ 子账户权限
│  └─ API Key管理
│
└─ 治理相关
   ├─ 手续费折扣规则
   ├─ VIP等级系统
   └─ 奖励分配
```

## 3. 状态管理深度解析

### 3.1 状态分层架构（Spot + Perp）

```
┌─────────────────────────────────────────────────────────────────────┐
│                       用户视图（聚合查询）                            │
├─────────────────────────┬───────────────────────────────────────────┤
│  Spot账户               │  Perp账户                                  │
│  - 总资产 = Σ币种余额    │  - 总资产 = 保证金 + 未实现盈亏              │
│  - BTC: 1.5             │  - 保证金: 50,000 USDT                    │
│  - USDT: 100,000        │  - 持仓: BTC-PERP +10 @ 50000             │
│  - ETH: 20.3            │  - 未实现盈亏: +5,000 USDT                 │
│                         │  - 可用保证金: 45,000 USDT                 │
└─────────────────────────┴───────────────────────────────────────────┘
                                    ↑
                          聚合查询接口（RPC）
                                    ↓
┌─────────────────────────┬───────────────────────────────────────────┐
│   EVM State             │   DEXVM State                             │
├─────────────────────────┼───────────────────────────────────────────┤
│ 【独占EVM】              │  【独占DEXVM】                             │
│ ├─ 策略合约存储          │  ├─ Spot订单簿（内存）                     │
│ │  └─ 网格配置          │  │  ├─ BTC/USDT买单: {...}                │
│ │  └─ 止损配置          │  │  └─ BTC/USDT卖单: {...}                │
│ ├─ 用户配置             │  ├─ Perp订单簿（内存）                     │
│ │  └─ 默认杠杆          │  │  ├─ BTC-PERP买单: {...}                │
│ │  └─ 通知设置          │  │  └─ BTC-PERP卖单: {...}                │
│ └─ 权限记录             │  ├─ Perp持仓（内存 + 持久化）               │
│    └─ 子账户权限        │  │  └─ User1: +10 BTC @ 50000             │
│                         │  ├─ 未成交订单索引                         │
│ 【共享 - 可转移】        │  └─ 撮合引擎状态                           │
│ ├─ Spot余额             │                                           │
│ │  ├─ BTC: 1.5         │  【共享 - 可转移】                          │
│ │  ├─ USDT: 100,000    │  ├─ Perp保证金                             │
│ │  └─ ETH: 20.3        │  │  └─ User1: 50,000 USDT                 │
│ │  (可转到Perp保证金)   │  │  (可转到Spot余额)                       │
│ └─ 冻结余额             │  └─ 冻结保证金                             │
│    └─ 挂单冻结          │     └─ 持仓占用                            │
└─────────────────────────┴───────────────────────────────────────────┘
                                    ↓
                  ┌─────────────────────────────────┐
                  │    Shared State Layer           │
                  ├─────────────────────────────────┤
                  │ - 账户Nonce（防重放）            │
                  │ - 用户资产索引（快速查询）        │
                  │ - 状态根（共识验证）              │
                  │ - 全局序列号（订单ID生成）        │
                  └─────────────────────────────────┘
                                    ↓
                  ┌─────────────────────────────────┐
                  │    Storage Layer (MDBX)         │
                  ├─────────────────────────────────┤
                  │ Tables:                         │
                  │ ├─ SpotBalances（Spot余额）      │
                  │ ├─ PerpPositions（Perp持仓）     │
                  │ ├─ PerpMargins（保证金）         │
                  │ ├─ ActiveOrders（活跃订单）      │
                  │ ├─ TradeHistory（成交历史）      │
                  │ ├─ UserConfigs（用户配置）       │
                  │ └─ LiquidationHistory（强平历史）│
                  └─────────────────────────────────┘
```

### 3.2 状态同步机制

#### 方案A：同步更新（简单但可能成为瓶颈）

```rust
/// 同步状态管理器
pub struct SyncStateManager {
    spot_state: SpotState,      // Spot余额
    perp_state: PerpState,       // Perp持仓和保证金
    evm_state: EvmState,         // EVM策略配置
    lock: RwLock<()>,            // 全局锁
}

impl SyncStateManager {
    /// Spot余额 → Perp保证金
    /// 用例：用户要开永续合约，需要从现货账户转保证金
    pub fn transfer_spot_to_perp(
        &mut self,
        user: Address,
        asset: Asset,
        amount: u128,
    ) -> Result<()> {
        // 【关键：获取全局锁】
        let _lock = self.lock.write();

        // 1. 检查Spot余额
        let spot_balance = self.spot_state.get_balance(user, asset)?;
        if spot_balance < amount {
            return Err(InsufficientBalance);
        }

        // 2. 扣除Spot余额
        self.spot_state.sub_balance(user, asset, amount)?;

        // 3. 增加Perp保证金
        self.perp_state.add_margin(user, asset, amount)?;

        // 4. 释放锁
        Ok(())
    }
}

// 问题分析：
// 1. 全局锁导致所有操作串行化
// 2. Spot交易阻塞Perp交易，反之亦然
// 3. 吞吐量受限于最慢的操作
//
// 性能影响：
// - 纯Spot交易: 200,000 TPS (DEXVM)
// - 纯Perp交易: 200,000 TPS (DEXVM)
// - 频繁Spot↔Perp转账: ~4,000 TPS ❌
//
// 结论：不可接受！只适合MVP测试
```

#### 方案B：异步更新（推荐）⭐

```rust
/// 异步状态管理器
pub struct AsyncStateManager {
    spot_state: Arc<SpotState>,
    perp_state: Arc<PerpState>,
    evm_state: Arc<EvmState>,

    /// 跨账户操作队列（Spot ↔ Perp）
    transfer_queue: Arc<SegQueue<TransferOp>>,

    /// 待处理的状态变更
    pending_changes: DashMap<TxHash, StateChange>,
}

/// 跨账户操作
#[derive(Debug)]
pub enum TransferOp {
    /// Spot → Perp保证金
    SpotToPerp {
        user: Address,
        asset: Asset,
        amount: u128,
        tx_hash: B256,
    },

    /// Perp保证金 → Spot
    PerpToSpot {
        user: Address,
        asset: Asset,
        amount: u128,
        tx_hash: B256,
    },
}

impl AsyncStateManager {
    /// 请求Spot → Perp转账
    /// 用例：用户从现货账户转USDT到合约账户做保证金
    pub fn request_spot_to_perp(
        &self,
        user: Address,
        asset: Asset,
        amount: u128,
        tx_hash: B256,
    ) -> Result<()> {
        // 【关键：立即扣除Spot余额，但标记为"待确认"】
        self.spot_state.sub_balance_pending(user, asset, amount, tx_hash)?;

        // 【异步队列：不阻塞Spot交易】
        self.transfer_queue.push(TransferOp::SpotToPerp {
            user,
            asset,
            amount,
            tx_hash,
        });

        // 立即返回，不等待Perp状态更新
        Ok(())
    }

    /// 后台工作线程：处理跨账户转账
    pub async fn transfer_worker(self: Arc<Self>) {
        loop {
            // 批量处理，提高吞吐量
            let mut batch = Vec::new();
            for _ in 0..1000 {
                if let Some(op) = self.transfer_queue.pop() {
                    batch.push(op);
                } else {
                    break;
                }
            }

            if batch.is_empty() {
                sleep(Duration::from_micros(100)).await;
                continue;
            }

            // 批量应用状态变更
            for op in batch {
                match op {
                    TransferOp::SpotToPerp { user, asset, amount, tx_hash } => {
                        // 【增加Perp保证金】
                        if let Err(e) = self.perp_state.add_margin(user, asset, amount) {
                            error!("Failed to add perp margin: {:?}", e);
                            // 回滚Spot状态
                            self.spot_state.refund_pending(user, asset, amount, tx_hash);
                        } else {
                            // 确认成功
                            self.spot_state.confirm_pending(tx_hash);
                        }
                    }

                    TransferOp::PerpToSpot { user, asset, amount, tx_hash } => {
                        // 反向操作：Perp → Spot
                        if let Err(e) = self.spot_state.add_balance(user, asset, amount) {
                            error!("Failed to add spot balance: {:?}", e);
                            self.perp_state.refund_pending(user, asset, amount, tx_hash);
                        } else {
                            self.perp_state.confirm_pending(tx_hash);
                        }
                    }
                }
            }
        }
    }

    /// 查询用户总资产（聚合视图）
    pub fn get_total_assets(&self, user: Address) -> UserAssets {
        // 【并行查询Spot和Perp】
        let (spot_assets, perp_assets) = rayon::join(
            || self.spot_state.get_all_balances(user),
            || self.perp_state.get_account_info(user),
        );

        UserAssets {
            spot: spot_assets,        // BTC: 1.5, USDT: 100000
            perp_margin: perp_assets.margin,  // 50000 USDT
            perp_unrealized_pnl: perp_assets.unrealized_pnl,  // +5000 USDT
            total: spot_assets.total + perp_assets.total(),
        }
    }
}

// 性能分析：
// - Spot交易：不受Perp影响，独立200k TPS
// - Perp交易：不受Spot影响，独立200k TPS
// - Spot↔Perp转账：异步处理，~1ms延迟
//
// 吞吐量（90% DEXVM纯交易 + 10% 跨账户转账）：
// - 纯Spot: 200,000 TPS
// - 纯Perp: 200,000 TPS
// - 混合（异步）: ~150,000 TPS ✅
//
// 结论：可接受！推荐生产环境使用
```

#### 方案C：分片隔离（最佳性能）

```rust
/// 分片状态管理器
/// 关键思想：按用户分片，避免全局竞争
pub struct ShardedStateManager {
    /// 每个分片独立管理一部分用户
    shards: Vec<Arc<ShardState>>,

    /// 用户到分片的映射
    router: ConsistentHashRouter,
}

pub struct ShardState {
    /// 分片ID
    shard_id: u32,

    /// 该分片的EVM状态
    evm_state: EvmState,

    /// 该分片的DEXVM状态
    dex_state: DexState,

    /// 分片内无需全局锁
    /// 只有跨分片操作才需要锁
}

impl ShardedStateManager {
    pub fn new(num_shards: usize) -> Self {
        let shards = (0..num_shards)
            .map(|id| Arc::new(ShardState::new(id as u32)))
            .collect();

        Self {
            shards,
            router: ConsistentHashRouter::new(num_shards),
        }
    }

    /// 转账到DEXVM（同分片）
    pub fn transfer_to_dex(
        &self,
        user: Address,
        asset: Asset,
        amount: u128,
    ) -> Result<()> {
        // 1. 路由到对应分片
        let shard_id = self.router.route(user);
        let shard = &self.shards[shard_id];

        // 2. 在分片内执行（无全局锁！）
        shard.local_transfer_to_dex(user, asset, amount)?;

        Ok(())
    }
}

impl ShardState {
    /// 分片内转账（无锁）
    fn local_transfer_to_dex(
        &self,
        user: Address,
        asset: Asset,
        amount: u128,
    ) -> Result<()> {
        // 关键：同一个分片的EVM和DEXVM状态在一起
        // 无需跨分片通信，无需全局锁

        self.evm_state.sub_balance(user, asset, amount)?;
        self.dex_state.add_margin(user, asset, amount)?;

        Ok(())
    }
}

// 性能分析：
// - 分片数量：16
// - 每分片EVM: 300 TPS
// - 每分片DEXVM: 12,500 TPS
// - 总吞吐量: 16 * 12,500 = 200,000 TPS ✅✅
//
// 优势：
// 1. 无全局锁竞争
// 2. 完美线性扩展
// 3. 分片间独立运行
//
// 挑战：
// 1. 跨分片转账复杂
// 2. 分片再平衡
// 3. 状态根计算复杂
```

### 3.3 状态一致性保证

```rust
/// 两阶段提交（2PC）：保证EVM和DEXVM原子性
pub struct TwoPhaseCommit {
    evm_state: Arc<EvmState>,
    dex_state: Arc<DexState>,
}

impl TwoPhaseCommit {
    /// 执行跨VM事务
    pub fn execute_cross_vm_tx(
        &self,
        tx: CrossVmTransaction,
    ) -> Result<()> {
        // ===== Phase 1: Prepare =====

        // 1. EVM准备
        let evm_prepared = self.evm_state.prepare(tx.evm_ops.clone())?;

        // 2. DEXVM准备
        let dex_prepared = self.dex_state.prepare(tx.dex_ops.clone())?;

        // 3. 检查是否都成功
        if !evm_prepared.can_commit() || !dex_prepared.can_commit() {
            // 任何一方失败，全部回滚
            evm_prepared.abort();
            dex_prepared.abort();
            return Err(TransactionFailed);
        }

        // ===== Phase 2: Commit =====

        // 4. 提交EVM
        evm_prepared.commit()?;

        // 5. 提交DEXVM
        dex_prepared.commit()?;

        Ok(())
    }
}

/// PreparedTransaction：支持两阶段提交
pub struct PreparedTransaction {
    state: Arc<dyn State>,
    changes: Vec<StateChange>,
    locks: Vec<Lock>,
    committed: bool,
}

impl PreparedTransaction {
    /// 检查是否可以提交
    pub fn can_commit(&self) -> bool {
        // 验证所有前置条件
        self.validate_preconditions() &&
        self.validate_locks() &&
        self.validate_balance_constraints()
    }

    /// 提交
    pub fn commit(mut self) -> Result<()> {
        // 应用所有状态变更
        for change in &self.changes {
            self.state.apply_change(change)?;
        }

        self.committed = true;
        Ok(())
    }

    /// 回滚
    pub fn abort(self) {
        // 释放所有锁，丢弃变更
        drop(self);
    }
}

impl Drop for PreparedTransaction {
    fn drop(&mut self) {
        if !self.committed {
            // 自动释放锁
            for lock in &self.locks {
                lock.release();
            }
        }
    }
}
```

### 3.4 实际性能测试场景

```rust
/// 压力测试：评估状态管理性能
#[tokio::test]
async fn benchmark_cross_vm_throughput() {
    let state_manager = AsyncStateManager::new();

    // 启动后台工作线程
    tokio::spawn(state_manager.clone().cross_vm_worker());

    // 场景1：纯EVM交易
    let evm_tps = measure_tps(|| {
        state_manager.execute_evm_tx(random_evm_tx());
    });
    println!("Pure EVM TPS: {}", evm_tps);
    // 预期：~5,000 TPS

    // 场景2：纯DEXVM交易
    let dex_tps = measure_tps(|| {
        state_manager.execute_dex_tx(random_dex_tx());
    });
    println!("Pure DEXVM TPS: {}", dex_tps);
    // 预期：~200,000 TPS

    // 场景3：10% EVM + 90% DEXVM
    let mixed_tps = measure_tps(|| {
        if random::<f32>() < 0.1 {
            state_manager.execute_evm_tx(random_evm_tx());
        } else {
            state_manager.execute_dex_tx(random_dex_tx());
        }
    });
    println!("Mixed (10/90) TPS: {}", mixed_tps);
    // 预期（异步）：~150,000 TPS
    // 预期（分片）：~190,000 TPS

    // 场景4：频繁跨VM转账（最坏情况）
    let cross_vm_tps = measure_tps(|| {
        state_manager.request_deposit_to_dex(
            random_address(),
            Asset::USDT,
            1000,
            random_hash(),
        );
        state_manager.execute_dex_tx(random_dex_tx());
    });
    println!("Cross-VM heavy TPS: {}", cross_vm_tps);
    // 预期（异步）：~50,000 TPS
    // 预期（分片）：~100,000 TPS
}
```

## 4. 性能瓶颈分析与解决方案

### 4.1 瓶颈识别

| 场景 | 瓶颈 | 影响 | 解决方案 |
|------|------|------|---------|
| **频繁跨VM转账** | 状态同步延迟 | TPS下降50% | 批量处理、异步队列 |
| **EVM调用预编译** | 序列化/反序列化开销 | 延迟+20μs | 零拷贝、共享内存 |
| **全局状态锁** | 并发受限 | TPS上限5k | 分片、无锁结构 |
| **状态根计算** | MPT遍历慢 | 出块慢1s | 增量计算、缓存 |
| **跨分片操作** | 需要协调 | 延迟+100μs | 减少跨分片、本地化 |

### 4.2 优化策略

#### 策略1：批量处理

```rust
/// 批量跨VM操作
pub struct BatchCrossVmProcessor {
    pending: Vec<CrossVmOp>,
    timer: Instant,
}

impl BatchCrossVmProcessor {
    pub fn add(&mut self, op: CrossVmOp) {
        self.pending.push(op);

        // 条件触发：数量或时间
        if self.pending.len() >= 1000 || self.timer.elapsed() > Duration::from_millis(10) {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }

        // 批量处理，减少锁竞争
        let batch = std::mem::take(&mut self.pending);

        // 并行执行
        batch.par_iter().for_each(|op| {
            process_cross_vm_op(op);
        });

        self.timer = Instant::now();
    }
}

// 性能提升：
// - 单个操作：100k TPS
// - 批量（1000）：500k TPS ✅
```

#### 策略2：零拷贝通信

```rust
/// 共享内存区域：EVM和DEXVM零拷贝通信
pub struct SharedMemoryRegion {
    /// mmap共享内存
    mmap: Arc<Mmap>,

    /// 环形缓冲区
    ring_buffer: Arc<RingBuffer>,
}

impl SharedMemoryRegion {
    /// EVM写入订单（零拷贝）
    pub fn write_order(&self, order: &Order) {
        // 直接写入共享内存
        let offset = self.ring_buffer.allocate(ORDER_SIZE);
        unsafe {
            let ptr = self.mmap.as_ptr().add(offset) as *mut Order;
            ptr.write(*order);
        }

        // 通知DEXVM（无需拷贝数据）
        self.ring_buffer.publish(offset);
    }

    /// DEXVM读取订单（零拷贝）
    pub fn read_order(&self) -> Option<&Order> {
        let offset = self.ring_buffer.consume()?;

        unsafe {
            let ptr = self.mmap.as_ptr().add(offset) as *const Order;
            Some(&*ptr)
        }
    }
}

// 性能提升：
// - 传统序列化：20μs
// - 零拷贝：2μs ✅
```

#### 策略3：预测性预加载

```rust
/// 智能预加载：预测用户下一步操作
pub struct PredictiveLoader {
    /// 用户行为模型
    user_patterns: DashMap<Address, UserPattern>,
}

impl PredictiveLoader {
    /// 分析用户行为
    pub fn analyze(&mut self, user: Address, action: Action) {
        let pattern = self.user_patterns.entry(user).or_default();
        pattern.record(action);

        // 预测下一步
        if let Some(next_action) = pattern.predict() {
            match next_action {
                PredictedAction::WillDepositToDex => {
                    // 提前准备DEXVM状态
                    self.prefetch_dex_state(user);
                }
                PredictedAction::WillTrade(symbol) => {
                    // 提前加载订单簿
                    self.prefetch_orderbook(symbol);
                }
                _ => {}
            }
        }
    }
}

// 性能提升：
// - 缓存命中：延迟减少30%
```

## 5. 总结与建议

### 5.1 业务逻辑划分原则（Spot + Perp）

```
核心原则：

【DEXVM - 20万TPS】
├─ Spot: 所有现货交易执行
│  └─ 下单、撤单、撮合、结算
├─ Perp: 所有合约交易执行
│  └─ 开仓、平仓、强平、资金费率
└─ 原则：能在DEXVM做的，绝不经过EVM

【EVM + 预编译 - 10-15万TPS】
├─ Spot策略: 网格、DCA、条件单
├─ Perp策略: 止盈止损、动态对冲
└─ 原则：决策在EVM，执行在DEXVM

【EVM - 5k TPS】
├─ 用户配置: 偏好设置、风控参数
├─ 权限管理: 子账户、API Key
└─ 原则：只存储配置，不执行交易

90%的性能来自10%的核心路径：
1. 优化DEXVM的纯交易流程（Spot/Perp）
2. 减少Spot↔Perp转账频率（用户提前分配资金）
3. 批量处理跨账户操作
4. 策略尽量简化，减少EVM调用
```

### 5.2 状态管理选择

| 方案 | 吞吐量 | 复杂度 | 延迟 | 推荐场景 |
|------|--------|--------|------|---------|
| 同步更新 | 4k TPS | 低 | 低 | MVP、测试 |
| 异步更新 | 150k TPS | 中 | 中 | 生产环境 |
| 分片隔离 | 200k TPS | 高 | 低 | 极致性能 |

**建议路线**：
1. Phase 1: 同步更新（快速上线）
2. Phase 2: 异步更新（满足大部分需求）
3. Phase 3: 分片隔离（如果需要极致性能）

### 5.3 避免性能陷阱

❌ **不要做的事**：
1. 频繁的小额Spot↔Perp转账
2. 在EVM策略中高频查询DEXVM（每次交易都查价格）
3. 全局锁保护Spot和Perp操作
4. 同步等待跨账户转账结果
5. 为了灵活性而牺牲性能（能用DEXVM别用EVM）

✅ **应该做的事**：
1. 用户一次性分配资金到Spot和Perp账户
2. 批量查询市场数据（缓存订单簿快照）
3. 异步处理Spot↔Perp转账
4. 按用户分片，隔离Spot和Perp状态
5. 简单交易用纯DEXVM，复杂策略才用EVM

### 5.4 最终建议

**对于20万TPS的Spot+Perp DEX目标**：

1. **用户分布预估**：
   - 80%用户：纯DEXVM直接交易（API、UI下单）
   - 15%用户：EVM策略交易（网格、止损）
   - 5%用户：复杂配置和管理

2. **架构选择**：
   - ✅ **双VM + 异步状态**（推荐）
     - Spot和Perp独立在DEXVM中执行
     - EVM提供策略和配置功能
     - 异步处理跨账户转账
     - 预期性能：15万TPS

   - ⭐ **双VM + 分片隔离**（追求极致）
     - 按用户分片Spot和Perp状态
     - 完美线性扩展
     - 预期性能：20万TPS
     - 适合大规模部署

3. **实施路线**：
   ```
   Phase 1 (MVP): 同步状态管理
   └─ 快速验证业务逻辑，性能~5k TPS

   Phase 2 (Beta): 异步状态管理
   └─ 生产可用，性能~15万TPS

   Phase 3 (Scale): 分片隔离
   └─ 大规模扩展，性能~20万TPS
   ```

4. **关键成功因素**：
   - 让80%+交易走纯DEXVM路径（不经过EVM）
   - 用户提前分配资金到Spot/Perp，减少跨账户转账
   - EVM策略保持简单，避免过度复杂的计算
   - 持续性能监控，识别并优化瓶颈

5. **业务设计建议**：
   ```
   用户入金 → 分配资金
   ├─ Spot账户: 70% （日常现货交易）
   └─ Perp账户: 30% （合约保证金）

   日常交易：
   ├─ 95%操作：纯DEXVM（下单、撤单、开平仓）
   ├─ 4%操作：EVM策略（网格、止损触发）
   └─ 1%操作：跨账户转账（异步处理）

   这样设计可达到接近纯DEXVM的性能
   ```
