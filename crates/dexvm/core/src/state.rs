//! DexVM 状态管理
//!
//! 管理账户状态和撮合引擎

use crate::matching_engine::MatchingEngine;
use reth_dexvm_primitives::{
    DexAccount, DexAccountState, DexInstruction, DexTransaction, DexVmError, ExecutionOutput,
    ExecutionResult, Order, OrderId, OrderSide, SignedDexTransaction, Trade, TradingPair,
};
use alloy_primitives::{Address, B256, U256};
use parking_lot::RwLock;
use std::sync::Arc;

/// DexVM 状态
pub struct DexVmState {
    /// 账户状态
    accounts: Arc<RwLock<DexAccountState>>,
    /// 撮合引擎
    matching_engine: Arc<MatchingEngine>,
    /// 当前时间戳（由区块执行器设置）
    current_timestamp: Arc<RwLock<u64>>,
}

impl DexVmState {
    /// 创建新状态
    pub fn new() -> Self {
        Self {
            accounts: Arc::new(RwLock::new(DexAccountState::new())),
            matching_engine: Arc::new(MatchingEngine::new()),
            current_timestamp: Arc::new(RwLock::new(0)),
        }
    }

    /// 设置当前时间戳
    pub fn set_timestamp(&self, timestamp: u64) {
        *self.current_timestamp.write() = timestamp;
    }

    /// 获取当前时间戳
    pub fn get_timestamp(&self) -> u64 {
        *self.current_timestamp.read()
    }

    /// 执行交易
    pub fn execute_transaction(
        &self,
        tx: &SignedDexTransaction,
    ) -> Result<ExecutionResult, DexVmError> {
        // 验证签名
        let sender = tx
            .verify_signature()
            .map_err(|_| DexVmError::InvalidSignature)?;

        if sender != tx.transaction.from {
            return Err(DexVmError::InvalidSignature);
        }

        // 验证 nonce
        {
            let accounts = self.accounts.read();
            if let Some(account) = accounts.get_account(&sender) {
                if account.nonce != tx.transaction.nonce {
                    return Err(DexVmError::InvalidNonce {
                        expected: account.nonce,
                        got: tx.transaction.nonce,
                    });
                }
            } else if tx.transaction.nonce != 0 {
                return Err(DexVmError::InvalidNonce {
                    expected: 0,
                    got: tx.transaction.nonce,
                });
            }
        }

        // 执行指令
        let result = self.execute_instruction(&sender, &tx.transaction.instruction)?;

        // 更新 nonce
        {
            let mut accounts = self.accounts.write();
            accounts.get_or_create_account(sender).increment_nonce();
        }

        Ok(result)
    }

    /// 执行指令
    fn execute_instruction(
        &self,
        sender: &Address,
        instruction: &DexInstruction,
    ) -> Result<ExecutionResult, DexVmError> {
        let gas_used = instruction.estimate_gas();

        match instruction {
            DexInstruction::PlaceLimitOrder {
                pair,
                side,
                price,
                amount,
            } => self.place_limit_order(sender, *pair, *side, *price, *amount, gas_used),

            DexInstruction::CancelOrder { order_id } => {
                self.cancel_order(sender, order_id, gas_used)
            }

            DexInstruction::Deposit { token, amount } => {
                self.deposit(sender, *token, *amount, gas_used)
            }

            DexInstruction::Withdraw { token, amount } => {
                self.withdraw(sender, *token, *amount, gas_used)
            }

            DexInstruction::QueryOrderBook { pair, depth } => {
                self.query_orderbook(*pair, *depth as usize, gas_used)
            }

            DexInstruction::QueryBalance { token } => self.query_balance(sender, *token, gas_used),

            DexInstruction::QueryOrder { order_id } => {
                self.query_order(*order_id, gas_used)
            }

            DexInstruction::Noop => Ok(ExecutionResult {
                success: true,
                gas_used: 0,
                output: ExecutionOutput::QueryResult(vec![]),
            }),
        }
    }

    /// 下限价单
    fn place_limit_order(
        &self,
        sender: &Address,
        pair: TradingPair,
        side: OrderSide,
        price: U256,
        amount: U256,
        gas_used: u64,
    ) -> Result<ExecutionResult, DexVmError> {
        if price.is_zero() || amount.is_zero() {
            return Err(DexVmError::InvalidOrder(
                "Price and amount must be non-zero".to_string(),
            ));
        }

        // 计算需要冻结的资产
        let token = pair.get_token_for_side(matches!(side, OrderSide::Buy));
        let freeze_amount = if matches!(side, OrderSide::Buy) {
            price * amount / U256::from(1e18 as u64) // 买单冻结计价币
        } else {
            amount // 卖单冻结基础币
        };

        // 冻结资产
        {
            let mut accounts = self.accounts.write();
            let account = accounts.get_or_create_account(*sender);
            account
                .freeze(token, freeze_amount)
                .map_err(|e| DexVmError::InsufficientBalance {
                    have: account.available_balance(&token),
                    need: freeze_amount,
                })?;
        }

        // 创建订单 (使用时间戳 + sender作为ID)
        let mut id_bytes = [0u8; 32];
        id_bytes[..20].copy_from_slice(sender.as_slice());
        id_bytes[20..28].copy_from_slice(&self.current_timestamp.read().to_le_bytes());
        let order_id = B256::from(id_bytes);
        let order = Order::new(
            order_id,
            pair,
            *sender,
            side,
            price,
            amount,
            self.get_timestamp(),
        );

        // 提交到撮合引擎
        let trades = self.matching_engine.place_order(order, self.get_timestamp());

        // 处理成交
        self.process_trades(&trades)?;

        Ok(ExecutionResult {
            success: true,
            gas_used,
            output: if trades.is_empty() {
                ExecutionOutput::OrderPlaced(order_id)
            } else {
                ExecutionOutput::TradesExecuted(trades)
            },
        })
    }

    /// 取消订单
    fn cancel_order(
        &self,
        sender: &Address,
        order_id: &OrderId,
        gas_used: u64,
    ) -> Result<ExecutionResult, DexVmError> {
        // 注意：需要遍历所有交易对查找订单，这里简化处理
        // 实际应该维护 user -> orders 的映射
        Err(DexVmError::Internal(
            "Cancel order not fully implemented".to_string(),
        ))
    }

    /// 充值
    fn deposit(
        &self,
        sender: &Address,
        token: Address,
        amount: U256,
        gas_used: u64,
    ) -> Result<ExecutionResult, DexVmError> {
        let mut accounts = self.accounts.write();
        let account = accounts.get_or_create_account(*sender);
        account.deposit(token, amount);

        Ok(ExecutionResult {
            success: true,
            gas_used,
            output: ExecutionOutput::QueryResult(vec![]),
        })
    }

    /// 提现
    fn withdraw(
        &self,
        sender: &Address,
        token: Address,
        amount: U256,
        gas_used: u64,
    ) -> Result<ExecutionResult, DexVmError> {
        let mut accounts = self.accounts.write();
        let account = accounts.get_or_create_account(*sender);
        account
            .withdraw(token, amount)
            .map_err(|_| DexVmError::InsufficientBalance {
                have: account.available_balance(&token),
                need: amount,
            })?;

        Ok(ExecutionResult {
            success: true,
            gas_used,
            output: ExecutionOutput::QueryResult(vec![]),
        })
    }

    /// 查询订单簿
    fn query_orderbook(
        &self,
        _pair: TradingPair,
        _depth: usize,
        gas_used: u64,
    ) -> Result<ExecutionResult, DexVmError> {
        // 简化实现：返回空结果
        let output = vec![];

        Ok(ExecutionResult {
            success: true,
            gas_used,
            output: ExecutionOutput::QueryResult(output),
        })
    }

    /// 查询余额
    fn query_balance(
        &self,
        sender: &Address,
        token: Address,
        gas_used: u64,
    ) -> Result<ExecutionResult, DexVmError> {
        let accounts = self.accounts.read();
        let balance = accounts
            .get_account(sender)
            .map(|acc| acc.available_balance(&token))
            .unwrap_or(U256::ZERO);

        // 简化实现：返回余额的字节表示
        let output = balance.to_be_bytes_vec();

        Ok(ExecutionResult {
            success: true,
            gas_used,
            output: ExecutionOutput::QueryResult(output),
        })
    }

    /// 查询订单
    fn query_order(
        &self,
        order_id: OrderId,
        gas_used: u64,
    ) -> Result<ExecutionResult, DexVmError> {
        // 简化实现
        Ok(ExecutionResult {
            success: true,
            gas_used,
            output: ExecutionOutput::QueryResult(vec![]),
        })
    }

    /// 处理成交
    fn process_trades(&self, trades: &[Trade]) -> Result<(), DexVmError> {
        let mut accounts = self.accounts.write();

        for trade in trades {
            let buyer_account = accounts.get_or_create_account(trade.buyer);
            let seller_account_addr = trade.seller;

            // 买方：扣除冻结的计价币，增加基础币
            let quote_amount = trade.price * trade.amount / U256::from(1e18 as u64);
            buyer_account
                .deduct_frozen(trade.pair.quote, quote_amount)
                .map_err(|e| DexVmError::Internal(e))?;
            buyer_account.deposit(trade.pair.base, trade.amount);

            // 卖方：扣除冻结的基础币，增加计价币
            let seller_account = accounts.get_or_create_account(seller_account_addr);
            seller_account
                .deduct_frozen(trade.pair.base, trade.amount)
                .map_err(|e| DexVmError::Internal(e))?;
            seller_account.deposit(trade.pair.quote, quote_amount);
        }

        Ok(())
    }

    /// 获取账户
    pub fn get_account(&self, address: &Address) -> Option<DexAccount> {
        self.accounts.read().get_account(address).cloned()
    }

    /// 获取账户状态（用于持久化）
    pub fn get_account_state(&self) -> DexAccountState {
        self.accounts.read().clone()
    }

    /// 加载账户状态
    pub fn load_account_state(&self, state: DexAccountState) {
        *self.accounts.write() = state;
    }
}

impl Default for DexVmState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, Signature};

    #[test]
    fn test_deposit_withdraw() {
        let state = DexVmState::new();
        let sender = address!("0000000000000000000000000000000000000001");
        let token = address!("0000000000000000000000000000000000000002");

        state.deposit(&sender, token, U256::from(1000), 0).unwrap();

        let account = state.get_account(&sender).unwrap();
        assert_eq!(account.available_balance(&token), U256::from(1000));

        state
            .withdraw(&sender, token, U256::from(300), 0)
            .unwrap();

        let account = state.get_account(&sender).unwrap();
        assert_eq!(account.available_balance(&token), U256::from(700));
    }
}
