//! 完整系统性能基准测试

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use reth_dexvm_bench::{generate_test_transactions, TestAccountGenerator};
use reth_dexvm_core::DexVmState;
use reth_dexvm_executor::{DexVmBlock, DexVmBlockExecutor};
use alloy_primitives::B256;
use std::sync::Arc;

/// 测试区块执行性能
fn bench_block_execution(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_execution");

    for tx_count in [100, 1_000, 10_000, 50_000] {
        group.throughput(Throughput::Elements(tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(tx_count),
            &tx_count,
            |b, &tx_count| {
                b.iter_batched(
                    || {
                        // Setup
                        let state = Arc::new(DexVmState::new());
                        let executor = DexVmBlockExecutor::new(state);
                        let mut account_gen = TestAccountGenerator::new();
                        let txs = generate_test_transactions(tx_count, &mut account_gen);

                        let mut block = DexVmBlock::new(1, 1000, B256::ZERO);
                        for tx in txs {
                            block.add_transaction(tx);
                        }

                        (executor, block)
                    },
                    |(executor, block)| {
                        let result = executor.execute_block(&block);
                        black_box(result);
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

/// 测试高负载下的 TPS
fn bench_sustained_tps(c: &mut Criterion) {
    let mut group = c.benchmark_group("sustained_tps");
    group.sample_size(10);

    for total_txs in [10_000, 50_000, 100_000] {
        group.throughput(Throughput::Elements(total_txs as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(total_txs),
            &total_txs,
            |b, &total_txs| {
                b.iter(|| {
                    let state = Arc::new(DexVmState::new());
                    let executor = DexVmBlockExecutor::new(state);
                    let mut account_gen = TestAccountGenerator::new();

                    let block_size = 10_000;
                    let num_blocks = total_txs / block_size;

                    for block_num in 0..num_blocks {
                        let txs = generate_test_transactions(block_size, &mut account_gen);
                        let mut block =
                            DexVmBlock::new(block_num as u64 + 1, 1000 * (block_num as u64 + 1), B256::ZERO);

                        for tx in txs {
                            block.add_transaction(tx);
                        }

                        let result = executor.execute_block(&block);
                        black_box(result);
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_block_execution, bench_sustained_tps);
criterion_main!(benches);
