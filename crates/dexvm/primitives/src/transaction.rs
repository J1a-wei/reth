//! DexVM 交易类型

use crate::instruction::DexInstruction;
use alloy_primitives::{Address, Signature, B256};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};

/// DexVM 交易（未签名）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
pub struct DexTransaction {
    /// 发送者
    pub from: Address,
    /// 指令
    #[rlp(default)]
    pub instruction: DexInstruction,
    /// Nonce（防重放）
    pub nonce: u64,
    /// Gas 限制
    pub gas_limit: u64,
    /// Gas 价格
    pub gas_price: u64,
}

impl DexTransaction {
    /// 创建新交易
    pub fn new(
        from: Address,
        instruction: DexInstruction,
        nonce: u64,
        gas_limit: u64,
        gas_price: u64,
    ) -> Self {
        Self {
            from,
            instruction,
            nonce,
            gas_limit,
            gas_price,
        }
    }

    /// 计算交易哈希
    pub fn hash(&self) -> B256 {
        use alloy_rlp::Encodable;
        let mut buf = Vec::new();
        self.encode(&mut buf);
        alloy_primitives::keccak256(&buf)
    }
}

/// 已签名的 DexVM 交易
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedDexTransaction {
    /// 交易内容
    pub transaction: DexTransaction,
    /// 签名
    pub signature: Signature,
    /// 交易哈希（缓存）
    #[serde(skip)]
    pub hash: Option<B256>,
}

impl SignedDexTransaction {
    /// 创建已签名交易
    pub fn new(transaction: DexTransaction, signature: Signature) -> Self {
        let hash = transaction.hash();
        Self {
            transaction,
            signature,
            hash: Some(hash),
        }
    }

    /// 获取交易哈希
    pub fn hash(&self) -> B256 {
        self.hash.unwrap_or_else(|| self.transaction.hash())
    }

    /// 验证签名
    pub fn verify_signature(&self) -> Result<Address, String> {
        let hash = self.transaction.hash();
        self.signature
            .recover_address_from_prehash(&hash)
            .map_err(|e| format!("Signature recovery failed: {}", e))
    }

    /// 获取发送者（从签名恢复）
    pub fn sender(&self) -> Result<Address, String> {
        self.verify_signature()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TradingPair;
    use alloy_primitives::{address, U256};

    #[test]
    fn test_transaction_hash() {
        let tx = DexTransaction::new(
            address!("0000000000000000000000000000000000000001"),
            DexInstruction::Deposit {
                token: address!("0000000000000000000000000000000000000002"),
                amount: U256::from(1000),
            },
            0,
            100_000,
            1,
        );

        let hash1 = tx.hash();
        let hash2 = tx.hash();
        assert_eq!(hash1, hash2);
    }
}
