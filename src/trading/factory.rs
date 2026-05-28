use std::sync::Arc;

use crate::instruction::{
    pumpfun::PumpFunInstructionBuilder, pumpswap::PumpSwapInstructionBuilder,
};

use super::core::{executor::GenericTradeExecutor, traits::TradeExecutor};

/// 支持的交易协议
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DexType {
    PumpFun,
    PumpSwap,
}

/// 交易工厂 - 用于创建不同协议的交易执行器
pub struct TradeFactory;

impl TradeFactory {
    /// 创建指定协议的交易执行器（零开销单例）
    pub fn create_executor(dex_type: DexType) -> Arc<dyn TradeExecutor> {
        match dex_type {
            DexType::PumpFun => Self::pumpfun_executor(),
            DexType::PumpSwap => Self::pumpswap_executor(),
        }
    }

    // Static instances created at compile time - zero runtime overhead
    #[inline]
    fn pumpfun_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(PumpFunInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "PumpFun"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn pumpswap_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(PumpSwapInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "PumpSwap"))
            });
        INSTANCE.clone()
    }
}
