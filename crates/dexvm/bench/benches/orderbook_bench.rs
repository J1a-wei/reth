//! 订单簿性能基准测试

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use reth_dexvm_bench::{random_order_tx, test_trading_pair, TestAccountGenerator};
use reth_dexvm_core::OrderBook;
use reth_dexvm_primitives::{Order, OrderSide};
use alloy_primitives::{address, B256, U256};

/// 测试订单簿添加订单性能
fn bench_orderbook_add_orders(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderbook_add");

    for size in [100, 1_000, 10_000, 100_000] {
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.iter(|| {
                let pair = test_trading_pair();
                let mut book = OrderBook::new(pair);

                for i in 0..size {
                    // Create deterministic order ID
                    let order_id = B256::left_padding_from(&(i as u64).to_le_bytes());
                    let order = Order::new(
                        order_id,
                        pair,
                        address!("1000000000000000000000000000000000000001"),
                        if i % 2 == 0 {
                            OrderSide::Buy
                        } else {
                            OrderSide::Sell
                        },
                        U256::from(50000 + (i % 100)),
                        U256::from(100),
                        i as u64,
                    );
                    book.add_order(order);
                }

                black_box(book);
            });
        });
    }

    group.finish();
}

/// 测试订撮合性能
fn bench_orderbook_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderbook_matching");

    for depth in [10, 100, 1_000] {
        group.throughput(Throughput::Elements(depth as u64));
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            b.iter_batched(
                || {
                    // Setup: 创建有深度的订单簿
                    let pair = test_trading_pair();
                    let mut book = OrderBook::new(pair);

                    // 添加卖单
                    for i in 0..depth {
                        let order_id = B256::left_padding_from(&((i + 1000) as u64).to_le_bytes());
                        let order = Order::new(
                            order_id,
                            pair,
                            address!("2000000000000000000000000000000000000001"),
                            OrderSide::Sell,
                            U256::from(50000 + i),
                            U256::from(100),
                            i as u64,
                        );
                        book.add_order(order);
                    }

                    // 创建大买单用于撮合
                    let taker_id = B256::left_padding_from(&(depth as u64 + 100000).to_le_bytes());
                    let taker_order = Order::new(
                        taker_id,
                        pair,
                        address!("3000000000000000000000000000000000000001"),
                        OrderSide::Buy,
                        U256::from(60000), // 高价买入
                        U256::from(depth as u64 * 50), // 足够大的数量
                        depth as u64,
                    );

                    (book, taker_order)
                },
                |(mut book, taker_order)| {
                    let trades = book.match_order(taker_order, depth as u64);
                    black_box(trades);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

/// 测试取消订单性能
fn bench_orderbook_cancel(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderbook_cancel");

    for size in [100, 1_000, 10_000] {
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let pair = test_trading_pair();
                    let mut book = OrderBook::new(pair);
                    let mut order_ids = Vec::new();

                    for i in 0..size {
                        let order_id = B256::left_padding_from(&((i + 200000) as u64).to_le_bytes());
                        order_ids.push(order_id);
                        let order = Order::new(
                            order_id,
                            pair,
                            address!("4000000000000000000000000000000000000001"),
                            OrderSide::Buy,
                            U256::from(50000),
                            U256::from(100),
                            i as u64,
                        );
                        book.add_order(order);
                    }

                    (book, order_ids)
                },
                |(mut book, order_ids)| {
                    for order_id in order_ids {
                        book.cancel_order(&order_id);
                    }
                    black_box(book);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_orderbook_add_orders,
    bench_orderbook_matching,
    bench_orderbook_cancel
);
criterion_main!(benches);
