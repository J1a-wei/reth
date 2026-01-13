# 双VM架构：EVM + DEXVM 混合方案

## 1. 架构概述

### 1.1 设计目标

- **兼容性**: 支持标准EVM智能合约，完全兼容以太坊生态
- **高性能**: DEX操作通过DEXVM实现，达到20万TPS
- **互操作性**: EVM合约可以通过预编译调用DEXVM功能
- **安全性**: 两个VM隔离运行，通过明确定义的接口交互

### 1.2 整体架构

```
┌─────────────────────────────────────────────────────────┐
│                    Application Layer                     │
│  ┌──────────────────┐         ┌──────────────────────┐  │
│  │  DeFi Contracts  │         │   DEX Trading UI     │  │
│  │  (Lending, etc)  │         │  (Order Management)  │  │
│  └──────────────────┘         └──────────────────────┘  │
└─────────────────────────────────────────────────────────┘
            │                              │
            ▼                              ▼
┌─────────────────────────────────────────────────────────┐
│                    Execution Layer                       │
│  ┌──────────────────────────────────────────────────┐  │
│  │              EVM (通用智能合约)                    │  │
│  │  - Solidity合约                                    │  │
│  │  - 通用DeFi逻辑                                    │  │
│  │  - ~5,000 TPS                                     │  │
│  └──────────────────────────────────────────────────┘  │
│                         │                               │
│                         ▼                               │
│  ┌──────────────────────────────────────────────────┐  │
│  │         Precompiled Contracts (Bridge)           │  │
│  │  0x8000: PlaceOrder                              │  │
│  │  0x8001: CancelOrder                             │  │
│  │  0x8002: QueryOrderBook                          │  │
│  │  0x8003: QueryPosition                           │  │
│  │  0x8004: TransferMargin                          │  │
│  └──────────────────────────────────────────────────┘  │
│                         │                               │
│                         ▼                               │
│  ┌──────────────────────────────────────────────────┐  │
│  │          DEXVM (专用订单撮合引擎)                   │  │
│  │  - 高性能订单簿                                    │  │
│  │  - 专用指令集                                      │  │
│  │  - ~200,000 TPS                                   │  │
│  └──────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│                    State Layer                           │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ EVM State    │  │ Shared State │  │ DEX State    │  │
│  │ - 合约存储   │  │ - 账户余额   │  │ - 订单簿     │  │
│  │ - 合约代码   │  │ - Nonce      │  │ - 持仓       │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
└─────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│               Storage (Reth Storage Layer)               │
│              MDBX + Static Files + In-Memory             │
└─────────────────────────────────────────────────────────┘
```

## 2. 预编译合约设计

### 2.1 预编译合约地址分配

```rust
/// DEX预编译合约地址范围: 0x8000 - 0x80FF
pub mod precompiles {
    use alloy_primitives::Address;

    /// 下单
    pub const PLACE_ORDER: Address = Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x00
    ]);

    /// 撤单
    pub const CANCEL_ORDER: Address = Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x01
    ]);

    /// 批量下单（高性能）
    pub const BATCH_PLACE_ORDERS: Address = Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x02
    ]);

    /// 批量撤单
    pub const BATCH_CANCEL_ORDERS: Address = Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x03
    ]);

    /// 查询订单簿（只读）
    pub const QUERY_ORDERBOOK: Address = Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x10
    ]);

    /// 查询持仓（只读）
    pub const QUERY_POSITION: Address = Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x11
    ]);

    /// 查询订单状态（只读）
    pub const QUERY_ORDER: Address = Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x12
    ]);

    /// 保证金转账（EVM <-> DEX）
    pub const TRANSFER_MARGIN: Address = Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x20
    ]);

    /// 更新风险参数
    pub const UPDATE_RISK_PARAMS: Address = Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x21
    ]);
}
```

### 2.2 预编译合约实现

```rust
use revm::precompile::{Precompile, PrecompileResult, PrecompileError};
use alloy_primitives::{Address, Bytes, U256};
use std::sync::Arc;

/// DEX预编译合约集合
pub struct DexPrecompiles {
    /// DEXVM实例
    dex_vm: Arc<DexVM>,
}

impl DexPrecompiles {
    /// 注册所有DEX预编译合约
    pub fn register(precompiles: &mut impl PrecompileRegistry) {
        precompiles.insert(PLACE_ORDER, Self::place_order_precompile());
        precompiles.insert(CANCEL_ORDER, Self::cancel_order_precompile());
        precompiles.insert(BATCH_PLACE_ORDERS, Self::batch_place_orders_precompile());
        precompiles.insert(QUERY_ORDERBOOK, Self::query_orderbook_precompile());
        precompiles.insert(TRANSFER_MARGIN, Self::transfer_margin_precompile());
        // ... 其他预编译
    }

    /// 下单预编译
    fn place_order_precompile() -> Precompile {
        Precompile::Standard(|input: &Bytes, gas_limit: u64| -> PrecompileResult {
            // Gas计费：基础成本
            const BASE_GAS: u64 = 50_000;
            if gas_limit < BASE_GAS {
                return Err(PrecompileError::OutOfGas);
            }

            // 解析输入
            let order = decode_order_input(input)?;

            // 调用DEXVM
            let result = DEXVM.place_order(order)?;

            // 编码返回值
            let output = encode_order_result(&result);

            Ok((BASE_GAS, output))
        })
    }

    /// 批量下单预编译（高性能路径）
    fn batch_place_orders_precompile() -> Precompile {
        Precompile::Standard(|input: &Bytes, gas_limit: u64| -> PrecompileResult {
            // 批量操作的gas计费
            let orders = decode_batch_orders(input)?;
            let gas_cost = 30_000 + orders.len() as u64 * 20_000;

            if gas_limit < gas_cost {
                return Err(PrecompileError::OutOfGas);
            }

            // 批量调用DEXVM（一次性提交，大幅提升性能）
            let results = DEXVM.batch_place_orders(orders)?;

            let output = encode_batch_results(&results);
            Ok((gas_cost, output))
        })
    }

    /// 查询订单簿（只读，gas很低）
    fn query_orderbook_precompile() -> Precompile {
        Precompile::Standard(|input: &Bytes, gas_limit: u64| -> PrecompileResult {
            const BASE_GAS: u64 = 3_000;
            if gas_limit < BASE_GAS {
                return Err(PrecompileError::OutOfGas);
            }

            let symbol = decode_symbol(input)?;
            let depth = decode_depth(input)?;

            // 只读查询DEXVM
            let orderbook = DEXVM.query_orderbook(symbol, depth)?;

            let output = encode_orderbook(&orderbook);
            Ok((BASE_GAS, output))
        })
    }

    /// 保证金转账（EVM <-> DEX）
    fn transfer_margin_precompile() -> Precompile {
        Precompile::Standard(|input: &Bytes, gas_limit: u64| -> PrecompileResult {
            const BASE_GAS: u64 = 40_000;
            if gas_limit < BASE_GAS {
                return Err(PrecompileError::OutOfGas);
            }

            let transfer = decode_margin_transfer(input)?;

            // 关键：需要同时更新EVM和DEX状态
            DEXVM.transfer_margin(transfer)?;

            Ok((BASE_GAS, Bytes::default()))
        })
    }
}
```

### 2.3 Solidity接口定义

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title IDexPrecompiles
/// @notice Interface for DEX precompiled contracts
library DexPrecompiles {
    /// @dev Precompile addresses
    address constant PLACE_ORDER = address(0x8000);
    address constant CANCEL_ORDER = address(0x8001);
    address constant BATCH_PLACE_ORDERS = address(0x8002);
    address constant BATCH_CANCEL_ORDERS = address(0x8003);
    address constant QUERY_ORDERBOOK = address(0x8010);
    address constant QUERY_POSITION = address(0x8011);
    address constant QUERY_ORDER = address(0x8012);
    address constant TRANSFER_MARGIN = address(0x8020);

    /// @notice Order side
    enum Side {
        Buy,
        Sell
    }

    /// @notice Order type
    enum OrderType {
        Limit,      // 限价单
        Market,     // 市价单
        PostOnly,   // 只做maker
        IOC,        // Immediate or Cancel
        FOK         // Fill or Kill
    }

    /// @notice Order struct (packed for gas efficiency)
    struct Order {
        bytes32 symbol;      // 交易对标识
        Side side;           // 买/卖
        OrderType orderType; // 订单类型
        uint128 price;       // 价格 (定点数)
        uint128 quantity;    // 数量
        uint64 leverage;     // 杠杆倍数 (1-100)
        bool reduceOnly;     // 只减仓
    }

    /// @notice Order result
    struct OrderResult {
        bytes32 orderId;     // 订单ID
        uint128 filledQty;   // 已成交数量
        uint128 avgPrice;    // 平均成交价
        uint8 status;        // 0=pending, 1=filled, 2=partial, 3=cancelled
    }

    /// @notice 下单
    /// @param order 订单参数
    /// @return result 订单结果
    function placeOrder(Order memory order)
        internal
        returns (OrderResult memory result)
    {
        bytes memory input = abi.encode(order);
        bytes memory output;

        assembly {
            let success := call(
                gas(),
                PLACE_ORDER,
                0,
                add(input, 0x20),
                mload(input),
                0,
                0
            )

            if iszero(success) {
                revert(0, 0)
            }

            // Copy return data
            let size := returndatasize()
            output := mload(0x40)
            mstore(output, size)
            returndatacopy(add(output, 0x20), 0, size)
            mstore(0x40, add(output, add(0x20, size)))
        }

        result = abi.decode(output, (OrderResult));
    }

    /// @notice 批量下单（高性能）
    /// @param orders 订单数组
    /// @return results 结果数组
    function batchPlaceOrders(Order[] memory orders)
        internal
        returns (OrderResult[] memory results)
    {
        bytes memory input = abi.encode(orders);
        bytes memory output;

        assembly {
            let success := call(
                gas(),
                BATCH_PLACE_ORDERS,
                0,
                add(input, 0x20),
                mload(input),
                0,
                0
            )

            if iszero(success) {
                revert(0, 0)
            }

            let size := returndatasize()
            output := mload(0x40)
            mstore(output, size)
            returndatacopy(add(output, 0x20), 0, size)
            mstore(0x40, add(output, add(0x20, size)))
        }

        results = abi.decode(output, (OrderResult[]));
    }

    /// @notice 撤单
    function cancelOrder(bytes32 orderId) internal returns (bool) {
        bytes memory input = abi.encode(orderId);

        assembly {
            let success := call(
                gas(),
                CANCEL_ORDER,
                0,
                add(input, 0x20),
                mload(input),
                0,
                0
            )

            if iszero(success) {
                revert(0, 0)
            }
        }

        return true;
    }

    /// @notice 查询订单簿
    struct OrderBookLevel {
        uint128 price;
        uint128 quantity;
    }

    struct OrderBook {
        OrderBookLevel[] bids;
        OrderBookLevel[] asks;
        uint128 lastPrice;
    }

    function queryOrderBook(bytes32 symbol, uint256 depth)
        internal
        view
        returns (OrderBook memory)
    {
        bytes memory input = abi.encode(symbol, depth);
        bytes memory output;

        assembly {
            let success := staticcall(
                gas(),
                QUERY_ORDERBOOK,
                add(input, 0x20),
                mload(input),
                0,
                0
            )

            if iszero(success) {
                revert(0, 0)
            }

            let size := returndatasize()
            output := mload(0x40)
            mstore(output, size)
            returndatacopy(add(output, 0x20), 0, size)
            mstore(0x40, add(output, add(0x20, size)))
        }

        return abi.decode(output, (OrderBook));
    }

    /// @notice 从EVM转保证金到DEX
    function depositMargin(bytes32 symbol, uint256 amount) internal {
        bytes memory input = abi.encode(symbol, amount, true); // true = deposit

        assembly {
            let success := call(
                gas(),
                TRANSFER_MARGIN,
                0,
                add(input, 0x20),
                mload(input),
                0,
                0
            )

            if iszero(success) {
                revert(0, 0)
            }
        }
    }

    /// @notice 从DEX提取保证金到EVM
    function withdrawMargin(bytes32 symbol, uint256 amount) internal {
        bytes memory input = abi.encode(symbol, amount, false); // false = withdraw

        assembly {
            let success := call(
                gas(),
                TRANSFER_MARGIN,
                0,
                add(input, 0x20),
                mload(input),
                0,
                0
            )

            if iszero(success) {
                revert(0, 0)
            }
        }
    }
}
```

### 2.4 使用示例合约

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "./DexPrecompiles.sol";

/// @title TradingStrategy
/// @notice 示例：在EVM中编写交易策略，通过预编译调用DEXVM
contract TradingStrategy {
    using DexPrecompiles for *;

    bytes32 public constant BTC_USD = keccak256("BTC/USD");
    bytes32 public constant ETH_USD = keccak256("ETH/USD");

    /// @notice 网格交易策略
    /// @param symbol 交易对
    /// @param basePrice 基准价格
    /// @param gridSize 网格大小
    /// @param levels 网格层数
    function gridTradingStrategy(
        bytes32 symbol,
        uint128 basePrice,
        uint128 gridSize,
        uint8 levels
    ) external {
        // 1. 构造多个订单
        DexPrecompiles.Order[] memory orders =
            new DexPrecompiles.Order[](levels * 2);

        for (uint8 i = 0; i < levels; i++) {
            // 买单
            orders[i * 2] = DexPrecompiles.Order({
                symbol: symbol,
                side: DexPrecompiles.Side.Buy,
                orderType: DexPrecompiles.OrderType.Limit,
                price: basePrice - gridSize * (i + 1),
                quantity: 1e18, // 1 unit
                leverage: 1,
                reduceOnly: false
            });

            // 卖单
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

        // 2. 批量下单（一次precompile调用，高效！）
        DexPrecompiles.OrderResult[] memory results =
            DexPrecompiles.batchPlaceOrders(orders);

        // 3. 处理结果
        for (uint i = 0; i < results.length; i++) {
            emit OrderPlaced(results[i].orderId, results[i].status);
        }
    }

    /// @notice 套利策略：跨交易对
    function arbitrageStrategy() external {
        // 1. 查询两个交易对的订单簿
        DexPrecompiles.OrderBook memory btcBook =
            DexPrecompiles.queryOrderBook(BTC_USD, 5);
        DexPrecompiles.OrderBook memory ethBook =
            DexPrecompiles.queryOrderBook(ETH_USD, 5);

        // 2. 计算套利机会（在EVM中进行复杂计算）
        (bool hasOpportunity, uint128 btcPrice, uint128 ethPrice) =
            calculateArbitrage(btcBook, ethBook);

        if (!hasOpportunity) return;

        // 3. 下套利订单
        DexPrecompiles.Order[] memory orders = new DexPrecompiles.Order[](2);
        orders[0] = DexPrecompiles.Order({
            symbol: BTC_USD,
            side: DexPrecompiles.Side.Buy,
            orderType: DexPrecompiles.OrderType.Market,
            price: 0, // 市价单
            quantity: 1e17, // 0.1 BTC
            leverage: 1,
            reduceOnly: false
        });
        orders[1] = DexPrecompiles.Order({
            symbol: ETH_USD,
            side: DexPrecompiles.Side.Sell,
            orderType: DexPrecompiles.OrderType.Market,
            price: 0,
            quantity: calculateEquivalent(1e17, btcPrice, ethPrice),
            leverage: 1,
            reduceOnly: false
        });

        DexPrecompiles.batchPlaceOrders(orders);
    }

    /// @notice 动态对冲策略
    function dynamicHedge() external {
        // EVM中的复杂逻辑...
    }

    function calculateArbitrage(
        DexPrecompiles.OrderBook memory book1,
        DexPrecompiles.OrderBook memory book2
    ) internal pure returns (bool, uint128, uint128) {
        // 复杂的套利计算逻辑
        // 这在EVM中运行，利用Solidity的表达能力
    }

    function calculateEquivalent(
        uint128 btcAmount,
        uint128 btcPrice,
        uint128 ethPrice
    ) internal pure returns (uint128) {
        // 计算等价数量
    }

    event OrderPlaced(bytes32 indexed orderId, uint8 status);
}

/// @title MarginManager
/// @notice 保证金管理合约：在EVM和DEX之间转移资金
contract MarginManager {
    using DexPrecompiles for *;

    mapping(address => uint256) public evmBalances;  // EVM余额
    mapping(address => uint256) public dexBalances;  // DEX保证金

    /// @notice 充值到DEX
    function depositToDex(bytes32 symbol, uint256 amount) external {
        require(evmBalances[msg.sender] >= amount, "Insufficient balance");

        // 1. 扣除EVM余额
        evmBalances[msg.sender] -= amount;

        // 2. 转入DEX（通过预编译）
        DexPrecompiles.depositMargin(symbol, amount);

        // 3. 记录DEX余额
        dexBalances[msg.sender] += amount;

        emit Deposit(msg.sender, symbol, amount);
    }

    /// @notice 从DEX提取
    function withdrawFromDex(bytes32 symbol, uint256 amount) external {
        require(dexBalances[msg.sender] >= amount, "Insufficient DEX balance");

        // 1. 从DEX提取（通过预编译）
        DexPrecompiles.withdrawMargin(symbol, amount);

        // 2. 增加EVM余额
        dexBalances[msg.sender] -= amount;
        evmBalances[msg.sender] += amount;

        emit Withdrawal(msg.sender, symbol, amount);
    }

    event Deposit(address indexed user, bytes32 symbol, uint256 amount);
    event Withdrawal(address indexed user, bytes32 symbol, uint256 amount);
}
```

## 3. DEXVM设计

### 3.1 DEXVM指令集

```rust
/// DEXVM专用指令集（字节码）
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum DexOpcode {
    // ===== 订单操作 (0x00 - 0x0F) =====
    /// PLACE_LIMIT: 限价单
    /// Stack: [symbol, side, price, quantity, leverage] -> [order_id]
    PlaceLimit = 0x00,

    /// PLACE_MARKET: 市价单
    /// Stack: [symbol, side, quantity, leverage] -> [order_id]
    PlaceMarket = 0x01,

    /// CANCEL: 撤单
    /// Stack: [order_id] -> [success]
    Cancel = 0x02,

    /// CANCEL_ALL: 撤销某交易对的所有订单
    /// Stack: [symbol] -> [count]
    CancelAll = 0x03,

    /// MODIFY: 修改订单
    /// Stack: [order_id, new_price, new_quantity] -> [success]
    Modify = 0x04,

    // ===== 查询操作 (0x10 - 0x1F) =====
    /// QUERY_ORDER: 查询订单状态
    /// Stack: [order_id] -> [status, filled_qty, avg_price]
    QueryOrder = 0x10,

    /// QUERY_POSITION: 查询持仓
    /// Stack: [symbol] -> [size, entry_price, unrealized_pnl, margin]
    QueryPosition = 0x11,

    /// QUERY_BALANCE: 查询余额
    /// Stack: [asset] -> [total, available, frozen]
    QueryBalance = 0x12,

    /// QUERY_ORDERBOOK: 查询订单簿
    /// Stack: [symbol, depth] -> [bids_ptr, asks_ptr, last_price]
    QueryOrderBook = 0x13,

    // ===== 保证金操作 (0x20 - 0x2F) =====
    /// DEPOSIT_MARGIN: 充值保证金
    /// Stack: [asset, amount] -> [success]
    DepositMargin = 0x20,

    /// WITHDRAW_MARGIN: 提取保证金
    /// Stack: [asset, amount] -> [success]
    WithdrawMargin = 0x21,

    /// TRANSFER_MARGIN: 转移保证金（跨交易对）
    /// Stack: [from_symbol, to_symbol, amount] -> [success]
    TransferMargin = 0x22,

    // ===== 风险管理 (0x30 - 0x3F) =====
    /// SET_LEVERAGE: 设置杠杆
    /// Stack: [symbol, leverage] -> [success]
    SetLeverage = 0x30,

    /// SET_STOP_LOSS: 设置止损
    /// Stack: [symbol, price] -> [success]
    SetStopLoss = 0x31,

    /// SET_TAKE_PROFIT: 设置止盈
    /// Stack: [symbol, price] -> [success]
    SetTakeProfit = 0x32,

    // ===== 栈操作 (0x40 - 0x4F) =====
    Push = 0x40,
    Pop = 0x41,
    Dup = 0x42,
    Swap = 0x43,

    // ===== 控制流 (0x50 - 0x5F) =====
    Jump = 0x50,
    JumpIf = 0x51,
    Call = 0x52,
    Return = 0x53,

    // ===== 算术操作 (0x60 - 0x6F) =====
    Add = 0x60,
    Sub = 0x61,
    Mul = 0x62,
    Div = 0x63,
    Mod = 0x64,

    // ===== 比较操作 (0x70 - 0x7F) =====
    Eq = 0x70,
    Lt = 0x71,
    Gt = 0x72,
    And = 0x73,
    Or = 0x74,

    // ===== 系统操作 (0xF0 - 0xFF) =====
    Halt = 0xFF,
}
```

### 3.2 DEXVM执行器

```rust
use alloy_primitives::{U256, B256};

/// DEXVM执行器
pub struct DexVM {
    /// 订单簿引擎
    matching_engine: Arc<MatchingEngine>,

    /// 状态管理器
    state: Arc<StateManager>,

    /// 指令缓存（JIT编译）
    jit_cache: DashMap<B256, CompiledProgram>,
}

/// DEXVM执行上下文
pub struct DexContext {
    /// 调用者地址
    caller: Address,

    /// 栈
    stack: Vec<U256>,

    /// 内存
    memory: Vec<u8>,

    /// 程序计数器
    pc: usize,

    /// Gas剩余
    gas_remaining: u64,
}

impl DexVM {
    /// 执行DEXVM字节码
    pub fn execute(
        &self,
        bytecode: &[u8],
        caller: Address,
        gas_limit: u64,
    ) -> Result<Vec<u8>, DexVMError> {
        let mut ctx = DexContext {
            caller,
            stack: Vec::with_capacity(1024),
            memory: Vec::new(),
            pc: 0,
            gas_remaining: gas_limit,
        };

        // 执行指令循环
        while ctx.pc < bytecode.len() {
            let opcode = DexOpcode::from_u8(bytecode[ctx.pc])?;
            ctx.pc += 1;

            self.execute_opcode(opcode, &mut ctx, bytecode)?;

            if matches!(opcode, DexOpcode::Halt | DexOpcode::Return) {
                break;
            }
        }

        Ok(ctx.memory)
    }

    /// 执行单个指令
    fn execute_opcode(
        &self,
        opcode: DexOpcode,
        ctx: &mut DexContext,
        bytecode: &[u8],
    ) -> Result<(), DexVMError> {
        match opcode {
            DexOpcode::PlaceLimit => {
                // Gas计费
                ctx.gas_remaining = ctx.gas_remaining
                    .checked_sub(GAS_PLACE_ORDER)
                    .ok_or(DexVMError::OutOfGas)?;

                // 从栈弹出参数
                let leverage = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                let quantity = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                let price = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                let side = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                let symbol = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;

                // 构造订单
                let order = Order {
                    symbol: B256::from(symbol),
                    side: if side.is_zero() { Side::Buy } else { Side::Sell },
                    order_type: OrderType::Limit,
                    price: price.to::<u128>(),
                    quantity: quantity.to::<u128>(),
                    leverage: leverage.to::<u8>(),
                    user: ctx.caller,
                    timestamp: SystemTime::now(),
                };

                // 执行下单（核心性能路径！）
                let order_id = self.matching_engine.place_order(order)?;

                // 结果压栈
                ctx.stack.push(U256::from_be_bytes(order_id.0));
            }

            DexOpcode::PlaceMarket => {
                // 类似实现
            }

            DexOpcode::Cancel => {
                ctx.gas_remaining = ctx.gas_remaining
                    .checked_sub(GAS_CANCEL_ORDER)
                    .ok_or(DexVMError::OutOfGas)?;

                let order_id = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                let success = self.matching_engine.cancel_order(
                    B256::from(order_id),
                    ctx.caller,
                )?;

                ctx.stack.push(U256::from(success as u64));
            }

            DexOpcode::QueryOrderBook => {
                // 只读操作，gas很低
                ctx.gas_remaining = ctx.gas_remaining
                    .checked_sub(GAS_QUERY_ORDERBOOK)
                    .ok_or(DexVMError::OutOfGas)?;

                let depth = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                let symbol = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;

                let orderbook = self.matching_engine.query_orderbook(
                    B256::from(symbol),
                    depth.to::<usize>(),
                )?;

                // 将订单簿写入内存
                let offset = ctx.memory.len();
                ctx.memory.extend_from_slice(&encode_orderbook(&orderbook));

                ctx.stack.push(U256::from(offset));
            }

            DexOpcode::DepositMargin => {
                ctx.gas_remaining = ctx.gas_remaining
                    .checked_sub(GAS_DEPOSIT)
                    .ok_or(DexVMError::OutOfGas)?;

                let amount = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                let asset = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;

                // 关键：从EVM状态转移到DEX状态
                self.state.deposit_from_evm(
                    ctx.caller,
                    B256::from(asset),
                    amount.to::<u128>(),
                )?;

                ctx.stack.push(U256::from(1u64));
            }

            // 栈操作
            DexOpcode::Push => {
                // 读取下一个32字节作为常量
                let value = U256::from_be_slice(&bytecode[ctx.pc..ctx.pc + 32]);
                ctx.pc += 32;
                ctx.stack.push(value);
                ctx.gas_remaining -= 3;
            }

            DexOpcode::Pop => {
                ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                ctx.gas_remaining -= 2;
            }

            DexOpcode::Dup => {
                let value = ctx.stack.last().ok_or(DexVMError::StackUnderflow)?;
                ctx.stack.push(*value);
                ctx.gas_remaining -= 3;
            }

            // 算术操作
            DexOpcode::Add => {
                let b = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                let a = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                ctx.stack.push(a + b);
                ctx.gas_remaining -= 3;
            }

            DexOpcode::Mul => {
                let b = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                let a = ctx.stack.pop().ok_or(DexVMError::StackUnderflow)?;
                ctx.stack.push(a * b);
                ctx.gas_remaining -= 5;
            }

            // 其他指令...
            _ => return Err(DexVMError::InvalidOpcode(opcode as u8)),
        }

        Ok(())
    }

    /// JIT编译（可选优化）
    pub fn jit_compile(&self, bytecode: &[u8]) -> CompiledProgram {
        // 将DEXVM字节码编译为本地机器码
        // 使用cranelift或LLVM
        // 可以带来2-5x性能提升
    }
}

/// Gas价格表
const GAS_PLACE_ORDER: u64 = 50_000;
const GAS_CANCEL_ORDER: u64 = 30_000;
const GAS_QUERY_ORDERBOOK: u64 = 3_000;
const GAS_QUERY_ORDER: u64 = 2_000;
const GAS_DEPOSIT: u64 = 40_000;
const GAS_WITHDRAW: u64 = 40_000;
```

### 3.3 DEXVM字节码示例

```rust
/// DEXVM字节码汇编器
pub struct DexAssembler {
    bytecode: Vec<u8>,
}

impl DexAssembler {
    /// 示例：下单程序
    ///
    /// ```dexasm
    /// PUSH symbol        // BTC/USD
    /// PUSH 0             // Side::Buy
    /// PUSH 50000         // price = $50,000
    /// PUSH 1000000000    // quantity = 1.0
    /// PUSH 10            // leverage = 10x
    /// PLACE_LIMIT        // 执行下单
    /// ```
    pub fn assemble_place_order() -> Vec<u8> {
        let mut asm = DexAssembler::new();

        // BTC/USD symbol
        asm.push(keccak256("BTC/USD"));
        // Side::Buy
        asm.push(U256::ZERO);
        // Price: 50000 USD
        asm.push(U256::from(50000u64 * 1e8 as u64));
        // Quantity: 1 BTC
        asm.push(U256::from(1e8 as u64));
        // Leverage: 10x
        asm.push(U256::from(10u64));
        // Place limit order
        asm.opcode(DexOpcode::PlaceLimit);
        // Return
        asm.opcode(DexOpcode::Return);

        asm.bytecode
    }

    /// 示例：复杂交易策略（用DEXVM编写）
    ///
    /// 逻辑：如果BTC价格 < $45000，买入；如果 > $55000，卖出
    pub fn assemble_conditional_strategy() -> Vec<u8> {
        let mut asm = DexAssembler::new();

        // 1. 查询当前价格
        asm.push(keccak256("BTC/USD"));
        asm.push(U256::from(1u64)); // depth = 1
        asm.opcode(DexOpcode::QueryOrderBook);
        // Stack: [orderbook_ptr]

        // 2. 从内存读取最新价格
        // (简化：假设返回的第一个值是最新价格)
        asm.push(U256::ZERO); // offset
        asm.opcode(DexOpcode::Load); // 自定义加载指令
        // Stack: [current_price]

        // 3. 比较价格并跳转
        asm.dup(); // 复制价格
        asm.push(U256::from(45000u64 * 1e8 as u64)); // $45,000
        asm.opcode(DexOpcode::Lt);
        // Stack: [current_price, is_below_45k]

        asm.push(U256::from(20u64)); // 跳转地址（买入分支）
        asm.opcode(DexOpcode::JumpIf);

        // 4. 检查是否高于$55,000（卖出条件）
        asm.push(U256::from(55000u64 * 1e8 as u64));
        asm.opcode(DexOpcode::Gt);
        // Stack: [is_above_55k]

        asm.push(U256::from(40u64)); // 跳转地址（卖出分支）
        asm.opcode(DexOpcode::JumpIf);

        // 5. 否则不操作
        asm.opcode(DexOpcode::Return);

        // 6. 买入分支 (offset 20)
        // PLACE BUY ORDER
        asm.push(keccak256("BTC/USD"));
        asm.push(U256::ZERO); // Buy
        asm.push(U256::from(45000u64 * 1e8 as u64));
        asm.push(U256::from(1e8 as u64));
        asm.push(U256::from(10u64));
        asm.opcode(DexOpcode::PlaceLimit);
        asm.opcode(DexOpcode::Return);

        // 7. 卖出分支 (offset 40)
        // PLACE SELL ORDER
        asm.push(keccak256("BTC/USD"));
        asm.push(U256::from(1u64)); // Sell
        asm.push(U256::from(55000u64 * 1e8 as u64));
        asm.push(U256::from(1e8 as u64));
        asm.push(U256::from(10u64));
        asm.opcode(DexOpcode::PlaceLimit);
        asm.opcode(DexOpcode::Return);

        asm.bytecode
    }

    fn push(&mut self, value: U256) {
        self.bytecode.push(DexOpcode::Push as u8);
        self.bytecode.extend_from_slice(&value.to_be_bytes::<32>());
    }

    fn opcode(&mut self, op: DexOpcode) {
        self.bytecode.push(op as u8);
    }

    fn dup(&mut self) {
        self.bytecode.push(DexOpcode::Dup as u8);
    }

    fn new() -> Self {
        Self { bytecode: Vec::new() }
    }
}
```

## 4. 状态管理与同步

### 4.1 共享状态层

```rust
/// 状态层：管理EVM和DEX的状态
pub struct HybridStateManager {
    /// EVM状态（标准的revm state）
    evm_state: EvmState,

    /// DEX状态
    dex_state: DexState,

    /// 共享账户信息（余额、nonce）
    accounts: DashMap<Address, AccountInfo>,

    /// 状态根（定期计算）
    state_root: B256,
}

/// 账户信息（EVM和DEX共享）
pub struct AccountInfo {
    /// 基础余额（EVM可用）
    balance: U256,

    /// DEX保证金（按交易对）
    margins: HashMap<Symbol, u128>,

    /// Nonce
    nonce: u64,

    /// EVM代码哈希（如果是合约）
    code_hash: B256,
}

impl HybridStateManager {
    /// 从EVM转账到DEX
    pub fn transfer_evm_to_dex(
        &mut self,
        user: Address,
        symbol: Symbol,
        amount: u128,
    ) -> Result<()> {
        // 1. 检查EVM余额
        let account = self.accounts.get_mut(&user)
            .ok_or(StateError::AccountNotFound)?;

        if account.balance < U256::from(amount) {
            return Err(StateError::InsufficientBalance);
        }

        // 2. 扣除EVM余额
        account.balance -= U256::from(amount);

        // 3. 增加DEX保证金
        *account.margins.entry(symbol).or_insert(0) += amount;

        // 4. 更新DEX状态
        self.dex_state.deposit_margin(user, symbol, amount)?;

        // 5. 记录状态变更（用于共识验证）
        self.state_changes.push(StateChange::EvmToDex {
            user,
            symbol,
            amount,
        });

        Ok(())
    }

    /// 从DEX转账到EVM
    pub fn transfer_dex_to_evm(
        &mut self,
        user: Address,
        symbol: Symbol,
        amount: u128,
    ) -> Result<()> {
        // 1. 检查DEX保证金
        let account = self.accounts.get_mut(&user)
            .ok_or(StateError::AccountNotFound)?;

        let margin = account.margins.get(&symbol)
            .ok_or(StateError::NoMargin)?;

        if *margin < amount {
            return Err(StateError::InsufficientMargin);
        }

        // 2. 检查持仓风险（不能提取有持仓的保证金）
        let position = self.dex_state.get_position(user, symbol)?;
        let required_margin = self.calculate_required_margin(&position)?;
        if margin - amount < required_margin {
            return Err(StateError::InsufficientMarginForPosition);
        }

        // 3. 扣除DEX保证金
        *account.margins.get_mut(&symbol).unwrap() -= amount;

        // 4. 增加EVM余额
        account.balance += U256::from(amount);

        // 5. 更新DEX状态
        self.dex_state.withdraw_margin(user, symbol, amount)?;

        // 6. 记录状态变更
        self.state_changes.push(StateChange::DexToEvm {
            user,
            symbol,
            amount,
        });

        Ok(())
    }

    /// 应用交易（同时更新EVM和DEX状态）
    pub fn apply_trade(&mut self, trade: &Trade) -> Result<()> {
        // 1. 更新买方
        self.update_position(&trade.buyer, trade.symbol, trade.quantity as i64, trade.price)?;

        // 2. 更新卖方
        self.update_position(&trade.seller, trade.symbol, -(trade.quantity as i64), trade.price)?;

        // 3. 结算资金
        self.settle_funds(trade)?;

        Ok(())
    }

    /// 计算统一状态根
    pub fn compute_state_root(&self) -> B256 {
        // 合并EVM和DEX状态计算统一的状态根
        // 选项1：分别计算后合并哈希
        let evm_root = self.evm_state.compute_root();
        let dex_root = self.dex_state.compute_root();
        keccak256(&[evm_root.as_slice(), dex_root.as_slice()].concat())

        // 选项2：使用统一的MPT（如果需要以太坊兼容性）
        // 选项3：使用Jellyfish Merkle Tree（性能更好）
    }
}
```

### 4.2 原子性保证

```rust
/// 事务：保证EVM和DEX状态的原子更新
pub struct HybridTransaction<'a> {
    state: &'a mut HybridStateManager,
    evm_changes: Vec<EvmStateChange>,
    dex_changes: Vec<DexStateChange>,
    committed: bool,
}

impl<'a> HybridTransaction<'a> {
    /// 创建事务
    pub fn begin(state: &'a mut HybridStateManager) -> Self {
        Self {
            state,
            evm_changes: Vec::new(),
            dex_changes: Vec::new(),
            committed: false,
        }
    }

    /// 执行EVM交易
    pub fn execute_evm(&mut self, tx: Transaction) -> Result<ExecutionResult> {
        // 执行EVM交易
        let result = self.state.evm_state.execute(tx)?;

        // 记录状态变更
        self.evm_changes.push(result.state_changes);

        Ok(result)
    }

    /// 执行DEX操作
    pub fn execute_dex(&mut self, op: DexOperation) -> Result<DexResult> {
        // 执行DEX操作
        let result = self.state.dex_state.execute(op)?;

        // 记录状态变更
        self.dex_changes.push(result.state_changes);

        Ok(result)
    }

    /// 提交事务（原子性）
    pub fn commit(mut self) -> Result<()> {
        // 1. 验证所有状态变更
        self.validate_changes()?;

        // 2. 原子应用所有变更
        for change in &self.evm_changes {
            self.state.apply_evm_change(change)?;
        }

        for change in &self.dex_changes {
            self.state.apply_dex_change(change)?;
        }

        // 3. 更新状态根
        self.state.state_root = self.state.compute_state_root();

        self.committed = true;
        Ok(())
    }

    /// 回滚事务
    pub fn rollback(self) {
        // 所有变更被丢弃
        drop(self);
    }
}

impl<'a> Drop for HybridTransaction<'a> {
    fn drop(&mut self) {
        if !self.committed {
            // 自动回滚未提交的事务
            warn!("Transaction dropped without commit, rolling back");
        }
    }
}
```

## 5. 执行流程

### 5.1 区块执行

```rust
/// 混合区块执行器
pub struct HybridBlockExecutor {
    evm_executor: EvmExecutor,
    dex_vm: Arc<DexVM>,
    state: Arc<Mutex<HybridStateManager>>,
}

impl HybridBlockExecutor {
    /// 执行区块
    pub fn execute_block(&self, block: &Block) -> Result<ExecutionResult> {
        let mut state = self.state.lock();
        let mut txn = HybridTransaction::begin(&mut state);

        let mut evm_gas_used = 0u64;
        let mut dex_ops_count = 0usize;
        let mut trades = Vec::new();

        // 处理区块中的所有交易
        for tx in &block.transactions {
            match tx {
                Transaction::Evm(evm_tx) => {
                    // EVM交易
                    let result = txn.execute_evm(evm_tx.clone())?;
                    evm_gas_used += result.gas_used;
                }

                Transaction::Dex(dex_tx) => {
                    // DEX交易（直接执行DEXVM字节码）
                    let result = self.dex_vm.execute(
                        &dex_tx.bytecode,
                        dex_tx.sender,
                        dex_tx.gas_limit,
                    )?;

                    dex_ops_count += 1;

                    // 如果是下单操作，可能产生成交
                    if let Some(new_trades) = self.extract_trades(&result) {
                        trades.extend(new_trades);
                    }
                }
            }
        }

        // 提交所有状态变更
        txn.commit()?;

        Ok(ExecutionResult {
            evm_gas_used,
            dex_ops_count,
            trades,
            state_root: state.state_root,
        })
    }
}
```

### 5.2 并行执行优化

```rust
/// 并行执行器：EVM和DEXVM可以并行处理
pub struct ParallelHybridExecutor {
    evm_executor: Arc<EvmExecutor>,
    dex_vm: Arc<DexVM>,
    state: Arc<HybridStateManager>,
}

impl ParallelHybridExecutor {
    /// 并行执行区块
    pub fn execute_block_parallel(&self, block: &Block) -> Result<ExecutionResult> {
        // 1. 分类交易
        let (evm_txs, dex_txs) = self.classify_transactions(&block.transactions);

        // 2. 并行执行（EVM和DEX没有依赖时）
        let (evm_result, dex_result) = rayon::join(
            || self.execute_evm_transactions(&evm_txs),
            || self.execute_dex_transactions(&dex_txs),
        );

        // 3. 合并结果
        let evm_result = evm_result?;
        let dex_result = dex_result?;

        // 4. 检测状态冲突
        if self.has_conflicts(&evm_result, &dex_result) {
            // 回滚并串行重试
            return self.execute_block_serial(block);
        }

        // 5. 合并状态
        self.merge_results(evm_result, dex_result)
    }

    /// 冲突检测：EVM和DEX是否访问了相同的账户
    fn has_conflicts(
        &self,
        evm_result: &EvmExecutionResult,
        dex_result: &DexExecutionResult,
    ) -> bool {
        let evm_touched: HashSet<_> = evm_result.touched_accounts.iter().collect();
        let dex_touched: HashSet<_> = dex_result.touched_accounts.iter().collect();

        // 如果有交集，说明存在冲突
        !evm_touched.is_disjoint(&dex_touched)
    }
}
```

## 6. 性能分析

### 6.1 TPS分解

```
假设配置：16核 64GB内存

纯EVM交易：~5,000 TPS
├── 签名验证: 20%
├── EVM执行: 50%
├── 状态更新: 20%
└── 共识: 10%

纯DEX交易（通过预编译）：~150,000 TPS
├── 签名验证: 10%
├── 预编译调用: 5%
├── DEXVM执行: 60%
├── 状态更新: 20%
└── 共识: 5%

混合场景（90% DEX + 10% EVM）：
= 0.9 * 150,000 + 0.1 * 5,000
= 135,000 + 500
= 135,500 TPS

性能提升来源：
1. DEXVM专用指令集，无需EVM的通用性开销
2. 批量操作（batchPlaceOrders）减少调用开销
3. 预编译避免了EVM解释执行
4. DEX状态简化，无需复杂的MPT
```

### 6.2 延迟分析

```
EVM合约调用DEX预编译的延迟：
├── EVM解释到预编译入口: ~10μs
├── 预编译参数解析: ~5μs
├── DEXVM执行: ~30μs
├── 状态更新: ~15μs
└── 返回到EVM: ~5μs
总计: ~65μs

直接DEXVM交易的延迟：
├── 签名验证: ~20μs
├── DEXVM执行: ~30μs
├── 状态更新: ~15μs
└── 网络传播: ~10μs
总计: ~75μs

对比：
- Hyperliquid: ~50μs (P50)
- 本方案: ~75μs (P50)
- 差距可接受，且有优化空间
```

## 7. 优势与劣势

### 7.1 优势

✅ **生态兼容性**
- 完整的EVM支持，兼容所有以太坊工具
- 可以部署标准的DeFi协议（Uniswap、Aave等）
- 钱包、开发工具无需修改

✅ **灵活性**
- 简单交易用EVM（易开发）
- 高频交易用DEXVM（高性能）
- 开发者可选择合适的VM

✅ **性能**
- DEX操作通过DEXVM，性能接近专用链
- EVM操作也有5k TPS，足够DeFi使用
- 混合场景可达10-15万TPS

✅ **渐进迁移**
- 可以先用EVM开发MVP
- 性能瓶颈再迁移到DEXVM
- 降低开发风险

### 7.2 劣势

❌ **复杂性**
- 需要维护两套VM
- 状态同步增加复杂度
- 调试和测试成本高

❌ **性能不如纯DEXVM**
- 混合架构有额外开销
- 状态同步和冲突检测
- 理论上限15万TPS vs 纯DEXVM的20万TPS

❌ **开发成本**
- 需要实现完整的DEXVM
- 预编译合约维护
- 需要更多的测试

### 7.3 适用场景

**推荐使用双VM的场景**
1. 需要同时支持通用DeFi和高性能DEX
2. 希望兼容以太坊生态
3. 用户可能需要复杂的智能合约策略
4. 项目早期，需要快速迭代

**不推荐使用双VM的场景**
1. 纯粹的DEX，不需要其他DeFi功能
2. 追求极致性能（20万+ TPS）
3. 团队规模小，无法维护复杂系统
4. 目标用户不需要智能合约

## 8. 实施建议

### 8.1 MVP路线（双VM）

**Phase 1: EVM only (1-2月)**
```
1. Fork reth
2. 实现基础DEX合约（Solidity）
3. 部署测试网
4. 验证功能和性能基线（~3-5k TPS）
```

**Phase 2: 添加预编译 (2-3月)**
```
1. 实现DEXVM核心（订单簿引擎）
2. 添加基础预编译合约（placeOrder, cancelOrder）
3. 集成到EVM执行流程
4. 性能测试（目标10-20k TPS）
```

**Phase 3: 完整DEXVM (2-3月)**
```
1. 实现完整的DEXVM指令集
2. 支持直接执行DEXVM字节码
3. 优化状态同步
4. 性能测试（目标50-100k TPS）
```

**Phase 4: 极致优化 (2-3月)**
```
1. JIT编译DEXVM
2. 并行执行优化
3. SIMD加速
4. 达到15万+ TPS
```

### 8.2 与纯DEXVM对比

| 维度 | 双VM方案 | 纯DEXVM方案 |
|------|---------|------------|
| **性能** | 10-15万TPS | 20万TPS |
| **EVM兼容** | ✅ 完全兼容 | ❌ 不兼容 |
| **开发成本** | 高（两套VM） | 中（一套VM） |
| **维护成本** | 高 | 中 |
| **灵活性** | 高 | 低 |
| **生态** | 可接入以太坊生态 | 需要重建生态 |
| **开发时间** | 8-11月 | 6-8月 |

### 8.3 最终建议

**如果你的目标是：**
- ✅ 兼容以太坊生态 → 选择双VM
- ✅ 支持复杂DeFi策略 → 选择双VM
- ✅ 快速上线MVP → 选择双VM（先EVM）

**如果你的目标是：**
- ❌ 极致性能（20万+ TPS） → 选择纯DEXVM
- ❌ 简单架构 → 选择纯DEXVM
- ❌ 类似Hyperliquid → 选择纯DEXVM

**我的建议：**
鉴于你提到"对标Hyperliquid"并且目标是20万TPS，我建议：
1. **优先选择纯DEXVM方案**（见第一份文档）
2. 如果后期发现需要EVM兼容，再添加EVM层
3. 先实现性能目标，再考虑生态兼容

理由：
- Hyperliquid不支持通用智能合约，但达到了10万TPS
- 双VM会牺牲一些性能，且复杂度高
- 可以通过跨链桥接入以太坊生态，无需原生EVM

**折中方案：**
如果确实需要智能合约支持，考虑：
1. 主链：纯DEXVM（20万TPS）
2. 侧链：EVM链（5k TPS）
3. 通过桥连接

这样可以兼得性能和灵活性，同时降低复杂度。
