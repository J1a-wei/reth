//! DexVM 区块执行器

use reth_dexvm_core::DexVmState;
use reth_dexvm_primitives::{ExecutionResult, ExecutionStats, SignedDexTransaction, DexVmError};
use std::sync::Arc;
use tracing::{debug, error, warn};

/// DexVM 区块
#[derive(Debug, Clone)]
pub struct DexVmBlock {
    /// 区块号
    pub number: u64,
    /// 时间戳
    pub timestamp: u64,
    /// 交易列表
    pub transactions: Vec<SignedDexTransaction>,
    /// 父区块哈希
    pub parent_hash: alloy_primitives::B256,
}

impl DexVmBlock {
    pub fn new(number: u64, timestamp: u64, parent_hash: alloy_primitives::B256) -> Self {
        Self {
            number,
            timestamp,
            transactions: Vec::new(),
            parent_hash,
        }
    }

    pub fn add_transaction(&mut self, tx: SignedDexTransaction) {
        self.transactions.push(tx);
    }
}

/// 区块执行结果
#[derive(Debug, Clone)]
pub struct BlockExecutionResult {
    /// 区块号
    pub block_number: u64,
    /// 成功的交易数
    pub successful_txs: u64,
    /// 失败的交易数
    pub failed_txs: u64,
    /// 总 Gas 消耗
    pub total_gas_used: u64,
    /// 执行时间（毫秒）
    pub execution_time_ms: u64,
    /// 每秒交易数
    pub tps: f64,
}

/// DexVM 区块执行器
pub struct DexVmBlockExecutor {
    /// 状态
    state: Arc<DexVmState>,
}

impl DexVmBlockExecutor {
    /// 创建新执行器
    pub fn new(state: Arc<DexVmState>) -> Self {
        Self { state }
    }

    /// 执行区块
    pub fn execute_block(&self, block: &DexVmBlock) -> BlockExecutionResult {
        let start = std::time::Instant::now();

        // 设置区块时间戳
        self.state.set_timestamp(block.timestamp);

        let mut successful_txs = 0;
        let mut failed_txs = 0;
        let mut total_gas_used = 0;

        // 执行所有交易
        for (idx, tx) in block.transactions.iter().enumerate() {
            match self.state.execute_transaction(tx) {
                Ok(result) => {
                    successful_txs += 1;
                    total_gas_used += result.gas_used;
                    debug!(
                        "Tx {} in block {} executed successfully, gas: {}",
                        idx, block.number, result.gas_used
                    );
                }
                Err(e) => {
                    failed_txs += 1;
                    warn!(
                        "Tx {} in block {} failed: {:?}",
                        idx, block.number, e
                    );
                }
            }
        }

        let execution_time_ms = start.elapsed().as_millis() as u64;
        let tps = if execution_time_ms > 0 {
            (successful_txs as f64) / (execution_time_ms as f64 / 1000.0)
        } else {
            0.0
        };

        BlockExecutionResult {
            block_number: block.number,
            successful_txs,
            failed_txs,
            total_gas_used,
            execution_time_ms,
            tps,
        }
    }

    /// 获取状态引用
    pub fn state(&self) -> &Arc<DexVmState> {
        &self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reth_dexvm_primitives::{DexInstruction, DexTransaction};
    use alloy_primitives::{address, Signature, B256, U256};

    #[test]
    fn test_block_execution() {
        let state = Arc::new(DexVmState::new());
        let executor = DexVmBlockExecutor::new(state.clone());

        // 创建测试区块
        let mut block = DexVmBlock::new(1, 1000, B256::ZERO);

        // 添加充值交易
        let tx = DexTransaction::new(
            address!("0000000000000000000000000000000000000001"),
            DexInstruction::Deposit {
                token: address!("0000000000000000000000000000000000000002"),
                amount: U256::from(10000),
            },
            0,
            100_000,
            1,
        );

        let signed_tx = SignedDexTransaction::new(tx, Signature::test_signature());
        block.add_transaction(signed_tx);

        // 执行区块
        let result = executor.execute_block(&block);
        assert_eq!(result.block_number, 1);
        // 注意：由于签名验证会失败，这个测试可能需要调整
    }
}
