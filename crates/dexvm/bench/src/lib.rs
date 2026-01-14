//! DexVM 性能测试工具库

use alloy_primitives::{address, Address, Signature, B256, U256};
use rand::Rng;
use reth_dexvm_primitives::{
    DexInstruction, DexTransaction, OrderSide, SignedDexTransaction, TradingPair,
};

/// 测试账户生成器
pub struct TestAccountGenerator {
    counter: u64,
}

impl TestAccountGenerator {
    pub fn new() -> Self {
        Self { counter: 0 }
    }

    pub fn next_address(&mut self) -> Address {
        self.counter += 1;
        let bytes = self.counter.to_be_bytes();
        let mut addr_bytes = [0u8; 20];
        addr_bytes[12..20].copy_from_slice(&bytes);
        Address::from_slice(&addr_bytes)
    }
}

impl Default for TestAccountGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// 测试交易对
pub fn test_trading_pair() -> TradingPair {
    TradingPair::new(
        address!("1111111111111111111111111111111111111111"), // BTC
        address!("2222222222222222222222222222222222222222"), // USDT
    )
}

/// 生成随机充值交易
pub fn random_deposit_tx(address: Address, nonce: u64) -> SignedDexTransaction {
    let token = address!("2222222222222222222222222222222222222222");
    let amount = U256::from(rand::thread_rng().gen_range(1000..100000));

    let tx = DexTransaction::new(
        address,
        DexInstruction::Deposit { token, amount },
        nonce,
        100_000,
        1,
    );

    SignedDexTransaction::new(tx, Signature::test_signature())
}

/// 生成随机下单交易
pub fn random_order_tx(
    address: Address,
    pair: TradingPair,
    side: OrderSide,
    nonce: u64,
) -> SignedDexTransaction {
    let mut rng = rand::thread_rng();
    let base_price = 50000;
    let price = U256::from(base_price + rng.gen_range(-1000..1000));
    let amount = U256::from(rng.gen_range(1..100));

    let tx = DexTransaction::new(
        address,
        DexInstruction::PlaceLimitOrder {
            pair,
            side,
            price,
            amount,
        },
        nonce,
        100_000,
        1,
    );

    SignedDexTransaction::new(tx, Signature::test_signature())
}

/// 批量生成测试交易
pub fn generate_test_transactions(
    count: usize,
    account_gen: &mut TestAccountGenerator,
) -> Vec<SignedDexTransaction> {
    let mut txs = Vec::with_capacity(count);
    let pair = test_trading_pair();

    for i in 0..count {
        let addr = account_gen.next_address();

        // 先充值
        txs.push(random_deposit_tx(addr, 0));

        // 然后随机买或卖
        let side = if i % 2 == 0 {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };
        txs.push(random_order_tx(addr, pair, side, 1));
    }

    txs
}
