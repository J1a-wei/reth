//! 简单的 DexVM 节点示例
//!
//! 演示如何启动单节点 DexVM

use reth_dexvm_core::DexVmState;
use reth_dexvm_executor::{DexVmBlockExecutor, SingleNodeProducer, TransactionPool};
use reth_dexvm_primitives::{DexInstruction, DexTransaction, OrderSide, SignedDexTransaction, TradingPair};
use alloy_primitives::{address, Signature, U256};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() {
    // 初始化日志
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    info!("Starting DexVM single node...");

    // 创建状态和执行器
    let state = Arc::new(DexVmState::new());
    let executor = Arc::new(DexVmBlockExecutor::new(state.clone()));
    let tx_pool = Arc::new(TransactionPool::new());

    // 创建出块器
    let producer = SingleNodeProducer::new(
        executor.clone(),
        tx_pool.clone(),
        Duration::from_secs(1), // 每秒出一个块
        10_000,                  // 每个块最多 10K 交易
    );

    // 启动出块
    let mut block_results = producer.start();

    info!("Node started, producing blocks every 1 second");

    // 模拟添加交易
    tokio::spawn({
        let tx_pool = tx_pool.clone();
        async move {
            let pair = TradingPair::new(
                address!("1111111111111111111111111111111111111111"), // BTC
                address!("2222222222222222222222222222222222222222"), // USDT
            );

            for i in 0..100 {
                tokio::time::sleep(Duration::from_millis(100)).await;

                let user = address!("3333333333333333333333333333333333333333");

                // 充值
                if i == 0 {
                    let deposit_tx = DexTransaction::new(
                        user,
                        DexInstruction::Deposit {
                            token: address!("2222222222222222222222222222222222222222"),
                            amount: U256::from(1_000_000),
                        },
                        0,
                        100_000,
                        1,
                    );
                    tx_pool
                        .add_transaction(SignedDexTransaction::new(deposit_tx, Signature::test_signature()))
                        .await;
                    info!("Added deposit transaction");
                }

                // 下单
                let order_tx = DexTransaction::new(
                    user,
                    DexInstruction::PlaceLimitOrder {
                        pair,
                        side: if i % 2 == 0 { OrderSide::Buy } else { OrderSide::Sell },
                        price: U256::from(50000 + i * 10),
                        amount: U256::from(1),
                    },
                    i + 1,
                    100_000,
                    1,
                );
                tx_pool
                    .add_transaction(SignedDexTransaction::new(order_tx, Signature::test_signature()))
                    .await;

                if i % 10 == 0 {
                    info!("Added {} transactions to pool", i + 1);
                }
            }
        }
    });

    // 接收区块执行结果
    let mut total_txs = 0;
    let mut total_blocks = 0;
    while let Some(result) = block_results.recv().await {
        total_blocks += 1;
        total_txs += result.successful_txs;

        info!(
            "📦 Block #{}: {} txs (✓{} ✗{}), {} ms, {:.2} TPS",
            result.block_number,
            result.successful_txs + result.failed_txs,
            result.successful_txs,
            result.failed_txs,
            result.execution_time_ms,
            result.tps
        );

        if total_blocks >= 10 {
            break;
        }
    }

    info!("Processed {} blocks with {} transactions", total_blocks, total_txs);
    info!("Shutting down...");
}
