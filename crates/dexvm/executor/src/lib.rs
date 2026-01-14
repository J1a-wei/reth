//! DexVM 执行器
//!
//! 处理交易和区块执行

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))] #![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod block_executor;
pub mod single_node;

pub use block_executor::{DexVmBlock, DexVmBlockExecutor};
pub use single_node::{SingleNodeProducer, TransactionPool};
