//! DexVM 账户模型

use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// DexVM 账户
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DexAccount {
    /// 账户地址
    pub address: Address,
    /// 资产余额 (token_address -> balance)
    pub balances: HashMap<Address, U256>,
    /// Nonce（防重放）
    pub nonce: u64,
    /// 冻结资产（下单后锁定）
    pub frozen: HashMap<Address, U256>,
}

impl DexAccount {
    /// 创建新账户
    pub fn new(address: Address) -> Self {
        Self {
            address,
            balances: HashMap::new(),
            nonce: 0,
            frozen: HashMap::new(),
        }
    }

    /// 获取可用余额
    pub fn available_balance(&self, token: &Address) -> U256 {
        let total = self.balances.get(token).copied().unwrap_or(U256::ZERO);
        let frozen = self.frozen.get(token).copied().unwrap_or(U256::ZERO);
        total.saturating_sub(frozen)
    }

    /// 充值
    pub fn deposit(&mut self, token: Address, amount: U256) {
        *self.balances.entry(token).or_insert(U256::ZERO) += amount;
    }

    /// 提现
    pub fn withdraw(&mut self, token: Address, amount: U256) -> Result<(), String> {
        let available = self.available_balance(&token);
        if available < amount {
            return Err(format!(
                "Insufficient balance: have {}, need {}",
                available, amount
            ));
        }
        *self.balances.entry(token).or_insert(U256::ZERO) -= amount;
        Ok(())
    }

    /// 冻结资产（下单时）
    pub fn freeze(&mut self, token: Address, amount: U256) -> Result<(), String> {
        let available = self.available_balance(&token);
        if available < amount {
            return Err(format!(
                "Insufficient balance to freeze: have {}, need {}",
                available, amount
            ));
        }
        *self.frozen.entry(token).or_insert(U256::ZERO) += amount;
        Ok(())
    }

    /// 解冻资产（取消订单时）
    pub fn unfreeze(&mut self, token: Address, amount: U256) {
        let frozen = self.frozen.entry(token).or_insert(U256::ZERO);
        *frozen = frozen.saturating_sub(amount);
    }

    /// 扣除冻结资产（成交时）
    pub fn deduct_frozen(&mut self, token: Address, amount: U256) -> Result<(), String> {
        let frozen = self.frozen.get_mut(&token).ok_or("No frozen balance")?;
        if *frozen < amount {
            return Err(format!(
                "Insufficient frozen balance: have {}, need {}",
                frozen, amount
            ));
        }
        *frozen -= amount;
        *self.balances.get_mut(&token).ok_or("Balance not found")? -= amount;
        Ok(())
    }

    /// 增加 nonce
    pub fn increment_nonce(&mut self) {
        self.nonce += 1;
    }
}

/// 账户状态（用于持久化）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexAccountState {
    /// 所有账户
    pub accounts: HashMap<Address, DexAccount>,
}

impl DexAccountState {
    /// 创建空状态
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
        }
    }

    /// 获取或创建账户
    pub fn get_or_create_account(&mut self, address: Address) -> &mut DexAccount {
        self.accounts
            .entry(address)
            .or_insert_with(|| DexAccount::new(address))
    }

    /// 获取账户
    pub fn get_account(&self, address: &Address) -> Option<&DexAccount> {
        self.accounts.get(address)
    }

    /// 获取可变账户
    pub fn get_account_mut(&mut self, address: &Address) -> Option<&mut DexAccount> {
        self.accounts.get_mut(address)
    }
}

impl Default for DexAccountState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn test_account_deposit_withdraw() {
        let addr = address!("0000000000000000000000000000000000000001");
        let token = address!("0000000000000000000000000000000000000002");
        let mut account = DexAccount::new(addr);

        account.deposit(token, U256::from(1000));
        assert_eq!(account.available_balance(&token), U256::from(1000));

        account.withdraw(token, U256::from(300)).unwrap();
        assert_eq!(account.available_balance(&token), U256::from(700));
    }

    #[test]
    fn test_freeze_unfreeze() {
        let addr = address!("0000000000000000000000000000000000000001");
        let token = address!("0000000000000000000000000000000000000002");
        let mut account = DexAccount::new(addr);

        account.deposit(token, U256::from(1000));
        account.freeze(token, U256::from(300)).unwrap();

        assert_eq!(account.available_balance(&token), U256::from(700));

        account.unfreeze(token, U256::from(100));
        assert_eq!(account.available_balance(&token), U256::from(800));
    }
}
