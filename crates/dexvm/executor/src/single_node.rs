//! 单节点出块器
//!
//! 定期打包交易到区块

use crate::block_executor::{BlockExecutionResult, DexVmBlock, DexVmBlockExecutor};
use reth_dexvm_primitives::SignedDexTransaction;
use alloy_primitives::B256;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

/// 交易池（简化版）
pub struct TransactionPool {
    pending: Arc<RwLock<Vec<SignedDexTransaction>>>,
}

impl TransactionPool {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// 添加交易
    pub async fn add_transaction(&self, tx: SignedDexTransaction) {
        self.pending.write().await.push(tx);
    }

    /// 获取待处理交易
    pub async fn get_pending(&self, limit: usize) -> Vec<SignedDexTransaction> {
        let mut pending = self.pending.write().await;
        let to_take = pending.len().min(limit);
        pending.drain(..to_take).collect()
    }

    /// 待处理交易数量
    pub async fn pending_count(&self) -> usize {
        self.pending.read().await.len()
    }
}

impl Default for TransactionPool {
    fn default() -> Self {
        Self::new()
    }
}

/// 单节点出块器
pub struct SingleNodeProducer {
    /// 区块执行器
    executor: Arc<DexVmBlockExecutor>,
    /// 交易池
    tx_pool: Arc<TransactionPool>,
    /// 当前区块号
    current_block: Arc<RwLock<u64>>,
    /// 上一个区块哈希
    last_hash: Arc<RwLock<B256>>,
    /// 出块间隔
    block_interval: Duration,
    /// 每个区块最大交易数
    max_txs_per_block: usize,
}

impl SingleNodeProducer {
    /// 创建新出块器
    pub fn new(
        executor: Arc<DexVmBlockExecutor>,
        tx_pool: Arc<TransactionPool>,
        block_interval: Duration,
        max_txs_per_block: usize,
    ) -> Self {
        Self {
            executor,
            tx_pool,
            current_block: Arc::new(RwLock::new(0)),
            last_hash: Arc::new(RwLock::new(B256::ZERO)),
            block_interval,
            max_txs_per_block,
        }
    }

    /// 启动出块（返回结果接收器）
    pub fn start(&self) -> mpsc::Receiver<BlockExecutionResult> {
        let (tx, rx) = mpsc::channel(100);
        let executor = self.executor.clone();
        let tx_pool = self.tx_pool.clone();
        let current_block = self.current_block.clone();
        let last_hash = self.last_hash.clone();
        let block_interval = self.block_interval;
        let max_txs_per_block = self.max_txs_per_block;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(block_interval);
            loop {
                interval.tick().await;

                // 获取待处理交易
                let pending_txs = tx_pool.get_pending(max_txs_per_block).await;
                if pending_txs.is_empty() {
                    continue;
                }

                // 创建区块
                let block_number = {
                    let mut num = current_block.write().await;
                    *num += 1;
                    *num
                };

                let parent_hash = *last_hash.read().await;
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();

                let mut block = DexVmBlock::new(block_number, timestamp, parent_hash);
                for tx in pending_txs {
                    block.add_transaction(tx);
                }

                info!(
                    "Producing block {} with {} transactions",
                    block_number,
                    block.transactions.len()
                );

                // 执行区块
                let result = executor.execute_block(&block);
                info!(
                    "Block {} executed: {} successful, {} failed, {} ms, {:.2} TPS",
                    result.block_number,
                    result.successful_txs,
                    result.failed_txs,
                    result.execution_time_ms,
                    result.tps
                );

                // 更新区块哈希（简化：使用区块号的哈希）
                *last_hash.write().await = B256::left_padding_from(&block_number.to_be_bytes());

                // 发送结果
                if tx.send(result).await.is_err() {
                    warn!("Failed to send block result, receiver dropped");
                    break;
                }
            }
        });

        rx
    }

    /// 获取当前区块号
    pub async fn current_block_number(&self) -> u64 {
        *self.current_block.read().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reth_dexvm_core::DexVmState;

    #[tokio::test]
    async fn test_transaction_pool() {
        let pool = TransactionPool::new();
        assert_eq!(pool.pending_count().await, 0);
    }
}
