use crate::common::bonding_curve::BondingCurveAccount;
use crate::common::nonce_cache::DurableNonceInfo;
use crate::common::spl_associated_token_account::get_associated_token_address_with_program_id;
use crate::common::{GasFeeStrategy, SolanaRpcClient};
use crate::instruction::utils::pumpfun::global_constants::MAYHEM_FEE_RECIPIENT;
use crate::instruction::utils::pumpswap::accounts::MAYHEM_FEE_RECIPIENT as MAYHEM_FEE_RECIPIENT_SWAP;
use crate::swqos::{SwqosClient, TradeType};
use crate::trading::MiddlewareManager;
use solana_hash::Hash;
use solana_sdk::message::AddressLookupTableAccount;
use solana_sdk::{pubkey::Pubkey, signature::Keypair};
use std::sync::Arc;

/// DEX 参数枚举 - 零开销抽象替代 Box<dyn ProtocolParams>
#[derive(Clone)]
pub enum DexParamEnum {
    PumpFun(PumpFunParams),
    PumpSwap(PumpSwapParams),
}

impl DexParamEnum {
    /// 获取内部参数的 Any 引用，用于向后兼容的类型检查
    #[inline]
    pub fn as_any(&self) -> &dyn std::any::Any {
        match self {
            DexParamEnum::PumpFun(p) => p,
            DexParamEnum::PumpSwap(p) => p,
        }
    }
}

/// Swap parameters
#[derive(Clone)]
pub struct SwapParams {
    pub rpc: Option<Arc<SolanaRpcClient>>,
    pub payer: Arc<Keypair>,
    pub trade_type: TradeType,
    pub input_mint: Pubkey,
    pub input_token_program: Option<Pubkey>,
    pub output_mint: Pubkey,
    pub output_token_program: Option<Pubkey>,
    pub input_amount: Option<u64>,
    pub slippage_basis_points: Option<u64>,
    pub address_lookup_table_account: Option<AddressLookupTableAccount>,
    pub recent_blockhash: Option<Hash>,
    pub wait_transaction_confirmed: bool,
    pub protocol_params: DexParamEnum,
    pub open_seed_optimize: bool,
    pub swqos_clients: Vec<Arc<SwqosClient>>,
    pub middleware_manager: Option<Arc<MiddlewareManager>>,
    pub durable_nonce: Option<DurableNonceInfo>,
    pub with_tip: bool,
    pub create_input_mint_ata: bool,
    pub close_input_mint_ata: bool,
    pub create_output_mint_ata: bool,
    pub close_output_mint_ata: bool,
    pub fixed_output_amount: Option<u64>,
    pub gas_fee_strategy: GasFeeStrategy,
    pub simulate: bool,
    /// Whether to output SDK logs (from TradeConfig.log_enabled).
    pub log_enabled: bool,
    /// Whether to pin parallel submit tasks to cores (from TradeConfig.use_core_affinity).
    pub use_core_affinity: bool,
    /// Whether to check minimum tip per SWQOS (from TradeConfig.check_min_tip). When false, skip filter for lower latency.
    pub check_min_tip: bool,
    /// Optional event receive time in microseconds (same scale as sol-parser-sdk clock::now_micros). Used as timing start when log_enabled.
    pub grpc_recv_us: Option<i64>,
    /// Use exact SOL amount instructions (buy_exact_sol_in for PumpFun, buy_exact_quote_in for PumpSwap).
    /// When Some(true) or None (default), the exact SOL/quote amount is spent and slippage is applied to output tokens.
    /// When Some(false), uses regular buy instruction where slippage is applied to SOL/quote input.
    /// This option only applies to PumpFun and PumpSwap DEXes; it is ignored for other DEXes.
    pub use_exact_sol_amount: Option<bool>,
}

impl std::fmt::Debug for SwapParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SwapParams: ...")
    }
}

/// PumpFun protocol specific parameters
/// Configuration parameters specific to PumpFun trading protocol
#[derive(Clone)]
pub struct PumpFunParams {
    pub bonding_curve: Arc<BondingCurveAccount>,
    pub associated_bonding_curve: Pubkey,
    pub creator_vault: Pubkey,
    pub token_program: Pubkey,
    /// Whether to close token account when selling, only effective during sell operations
    pub close_token_account_when_sell: Option<bool>,
}

impl PumpFunParams {
    pub fn immediate_sell(
        creator_vault: Pubkey,
        token_program: Pubkey,
        close_token_account_when_sell: bool,
    ) -> Self {
        Self {
            bonding_curve: Arc::new(BondingCurveAccount { ..Default::default() }),
            associated_bonding_curve: Pubkey::default(),
            creator_vault: creator_vault,
            token_program: token_program,
            close_token_account_when_sell: Some(close_token_account_when_sell),
        }
    }

    /// When building from event/parser (e.g. sol-parser-sdk), pass `is_cashback_coin` from the event
    /// so that sell instructions include the correct remaining accounts for cashback.
    pub fn from_dev_trade(
        mint: Pubkey,
        token_amount: u64,
        max_sol_cost: u64,
        creator: Pubkey,
        bonding_curve: Pubkey,
        associated_bonding_curve: Pubkey,
        creator_vault: Pubkey,
        close_token_account_when_sell: Option<bool>,
        fee_recipient: Pubkey,
        token_program: Pubkey,
        is_cashback_coin: bool,
    ) -> Self {
        let is_mayhem_mode = fee_recipient == MAYHEM_FEE_RECIPIENT;
        let bonding_curve_account = BondingCurveAccount::from_dev_trade(
            bonding_curve,
            &mint,
            token_amount,
            max_sol_cost,
            creator,
            is_mayhem_mode,
            is_cashback_coin,
        );
        Self {
            bonding_curve: Arc::new(bonding_curve_account),
            associated_bonding_curve: associated_bonding_curve,
            creator_vault: creator_vault,
            close_token_account_when_sell: close_token_account_when_sell,
            token_program: token_program,
        }
    }

    /// When building from event/parser (e.g. sol-parser-sdk), pass `is_cashback_coin` from the event
    /// so that sell instructions include the correct remaining accounts for cashback.
    pub fn from_trade(
        bonding_curve: Pubkey,
        associated_bonding_curve: Pubkey,
        mint: Pubkey,
        creator: Pubkey,
        creator_vault: Pubkey,
        virtual_token_reserves: u64,
        virtual_sol_reserves: u64,
        real_token_reserves: u64,
        real_sol_reserves: u64,
        close_token_account_when_sell: Option<bool>,
        fee_recipient: Pubkey,
        token_program: Pubkey,
        is_cashback_coin: bool,
    ) -> Self {
        let is_mayhem_mode = fee_recipient == MAYHEM_FEE_RECIPIENT;
        let bonding_curve = BondingCurveAccount::from_trade(
            bonding_curve,
            mint,
            creator,
            virtual_token_reserves,
            virtual_sol_reserves,
            real_token_reserves,
            real_sol_reserves,
            is_mayhem_mode,
            is_cashback_coin,
        );
        Self {
            bonding_curve: Arc::new(bonding_curve),
            associated_bonding_curve: associated_bonding_curve,
            creator_vault: creator_vault,
            close_token_account_when_sell: close_token_account_when_sell,
            token_program: token_program,
        }
    }

    pub async fn from_mint_by_rpc(
        rpc: &SolanaRpcClient,
        mint: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let account =
            crate::instruction::utils::pumpfun::fetch_bonding_curve_account(rpc, mint).await?;
        let mint_account = rpc.get_account(&mint).await?;
        let bonding_curve = BondingCurveAccount {
            discriminator: 0,
            account: account.1,
            virtual_token_reserves: account.0.virtual_token_reserves,
            virtual_sol_reserves: account.0.virtual_sol_reserves,
            real_token_reserves: account.0.real_token_reserves,
            real_sol_reserves: account.0.real_sol_reserves,
            token_total_supply: account.0.token_total_supply,
            complete: account.0.complete,
            creator: account.0.creator,
            is_mayhem_mode: account.0.is_mayhem_mode,
            is_cashback_coin: account.0.is_cashback_coin,
        };
        let associated_bonding_curve = get_associated_token_address_with_program_id(
            &bonding_curve.account,
            mint,
            &mint_account.owner,
        );
        let creator_vault =
            crate::instruction::utils::pumpfun::get_creator_vault_pda(&bonding_curve.creator);
        Ok(Self {
            bonding_curve: Arc::new(bonding_curve),
            associated_bonding_curve: associated_bonding_curve,
            creator_vault: creator_vault.unwrap(),
            close_token_account_when_sell: None,
            token_program: mint_account.owner,
        })
    }
}

/// PumpSwap Protocol Specific Parameters
///
/// Parameters for configuring PumpSwap trading protocol, including liquidity pool information,
/// token configuration, and transaction amounts.
///
/// **Performance Note**: If these parameters are not provided, the system will attempt to
/// retrieve the relevant information from RPC, which will increase transaction time.
/// For optimal performance, it is recommended to provide all necessary parameters in advance.
#[derive(Clone)]
pub struct PumpSwapParams {
    /// Liquidity pool address
    pub pool: Pubkey,
    /// Base token mint address
    /// The mint account address of the base token in the trading pair
    pub base_mint: Pubkey,
    /// Quote token mint address
    /// The mint account address of the quote token in the trading pair, usually SOL or USDC
    pub quote_mint: Pubkey,
    /// Pool base token account
    pub pool_base_token_account: Pubkey,
    /// Pool quote token account
    pub pool_quote_token_account: Pubkey,
    /// Base token reserves in the pool
    pub pool_base_token_reserves: u64,
    /// Quote token reserves in the pool
    pub pool_quote_token_reserves: u64,
    /// Coin creator vault ATA
    pub coin_creator_vault_ata: Pubkey,
    /// Coin creator vault authority
    pub coin_creator_vault_authority: Pubkey,
    /// Token program ID
    pub base_token_program: Pubkey,
    /// Quote token program ID
    pub quote_token_program: Pubkey,
    /// Whether the pool is in mayhem mode
    pub is_mayhem_mode: bool,
    /// Whether the pool's coin has cashback enabled
    pub is_cashback_coin: bool,
}

impl PumpSwapParams {
    pub fn new(
        pool: Pubkey,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        pool_base_token_account: Pubkey,
        pool_quote_token_account: Pubkey,
        pool_base_token_reserves: u64,
        pool_quote_token_reserves: u64,
        coin_creator_vault_ata: Pubkey,
        coin_creator_vault_authority: Pubkey,
        base_token_program: Pubkey,
        quote_token_program: Pubkey,
        fee_recipient: Pubkey,
        is_cashback_coin: bool,
    ) -> Self {
        let is_mayhem_mode = fee_recipient == MAYHEM_FEE_RECIPIENT_SWAP;
        Self {
            pool,
            base_mint,
            quote_mint,
            pool_base_token_account,
            pool_quote_token_account,
            pool_base_token_reserves,
            pool_quote_token_reserves,
            coin_creator_vault_ata,
            coin_creator_vault_authority,
            base_token_program,
            quote_token_program,
            is_mayhem_mode,
            is_cashback_coin,
        }
    }

    /// Fast-path constructor for building PumpSwap parameters directly from decoded
    /// trade/event data and the accompanying instruction accounts, avoiding RPC
    /// lookups and associated latency. Token program IDs should be sourced from
    /// the instruction accounts themselves to respect Token Program vs Token-2022
    /// differences.
    ///
    /// When building from event/parser (e.g. sol-parser-sdk), pass `is_cashback_coin`
    /// from the event so that buy/sell instructions include the correct remaining
    /// accounts for cashback.
    pub fn from_trade(
        pool: Pubkey,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        pool_base_token_account: Pubkey,
        pool_quote_token_account: Pubkey,
        pool_base_token_reserves: u64,
        pool_quote_token_reserves: u64,
        coin_creator_vault_ata: Pubkey,
        coin_creator_vault_authority: Pubkey,
        base_token_program: Pubkey,
        quote_token_program: Pubkey,
        fee_recipient: Pubkey,
        is_cashback_coin: bool,
    ) -> Self {
        Self::new(
            pool,
            base_mint,
            quote_mint,
            pool_base_token_account,
            pool_quote_token_account,
            pool_base_token_reserves,
            pool_quote_token_reserves,
            coin_creator_vault_ata,
            coin_creator_vault_authority,
            base_token_program,
            quote_token_program,
            fee_recipient,
            is_cashback_coin,
        )
    }

    pub async fn from_mint_by_rpc(
        rpc: &SolanaRpcClient,
        mint: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        if let Ok((pool_address, _)) =
            crate::instruction::utils::pumpswap::find_by_base_mint(rpc, mint).await
        {
            Self::from_pool_address_by_rpc(rpc, &pool_address).await
        } else if let Ok((pool_address, _)) =
            crate::instruction::utils::pumpswap::find_by_quote_mint(rpc, mint).await
        {
            Self::from_pool_address_by_rpc(rpc, &pool_address).await
        } else {
            return Err(anyhow::anyhow!("No pool found for mint"));
        }
    }

    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let pool_data = crate::instruction::utils::pumpswap::fetch_pool(rpc, pool_address).await?;
        let (pool_base_token_reserves, pool_quote_token_reserves) =
            crate::instruction::utils::pumpswap::get_token_balances(&pool_data, rpc).await?;
        let creator = pool_data.coin_creator;
        let coin_creator_vault_ata = crate::instruction::utils::pumpswap::coin_creator_vault_ata(
            creator,
            pool_data.quote_mint,
        );
        let coin_creator_vault_authority =
            crate::instruction::utils::pumpswap::coin_creator_vault_authority(creator);

        let base_token_program_ata = get_associated_token_address_with_program_id(
            &pool_address,
            &pool_data.base_mint,
            &crate::constants::TOKEN_PROGRAM,
        );
        let quote_token_program_ata = get_associated_token_address_with_program_id(
            &pool_address,
            &pool_data.quote_mint,
            &crate::constants::TOKEN_PROGRAM,
        );

        Ok(Self {
            pool: pool_address.clone(),
            base_mint: pool_data.base_mint,
            quote_mint: pool_data.quote_mint,
            pool_base_token_account: pool_data.pool_base_token_account,
            pool_quote_token_account: pool_data.pool_quote_token_account,
            pool_base_token_reserves: pool_base_token_reserves,
            pool_quote_token_reserves: pool_quote_token_reserves,
            coin_creator_vault_ata: coin_creator_vault_ata,
            coin_creator_vault_authority: coin_creator_vault_authority,
            base_token_program: if pool_data.pool_base_token_account == base_token_program_ata {
                crate::constants::TOKEN_PROGRAM
            } else {
                crate::constants::TOKEN_PROGRAM_2022
            },
            is_cashback_coin: pool_data.is_cashback_coin,
            quote_token_program: if pool_data.pool_quote_token_account == quote_token_program_ata {
                crate::constants::TOKEN_PROGRAM
            } else {
                crate::constants::TOKEN_PROGRAM_2022
            },
            is_mayhem_mode: pool_data.is_mayhem_mode,
        })
    }
}
