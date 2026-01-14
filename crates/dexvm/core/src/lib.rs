//! DexVM 核心引擎
//!
//! 包含订单簿、撮合引擎、状态管理等核心组件

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod matching_engine;
pub mod orderbook;
pub mod state;

pub use matching_engine::MatchingEngine;
pub use orderbook::{OrderBook, PriceLevel};
pub use state::DexVmState;
