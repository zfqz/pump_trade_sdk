use std::{str::FromStr, sync::Arc};

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::Local;
use clap::{Parser, Subcommand};
use log::{Level, LevelFilter, Metadata, Record};
use serde::Deserialize;
use sol_trade_sdk::{
    common::{
        fast_fn::get_associated_token_address_with_program_id_fast_use_seed,
        nonce_cache::fetch_nonce_info, GasFeeStrategy, TradeConfig,
    },
    swqos::{SwqosConfig, SwqosRegion},
    trading::{
        core::params::{DexParamEnum, PumpFunParams, PumpSwapParams},
        factory::DexType,
    },
    SolanaTrade, TradeTokenType,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
};
use x_sol::core::{
    engine::Engine,
    program_ids,
    types::{Action, CollectorMap, Event, ExecutorMap},
    utils::sol_to_lamports,
};

const XOR_KEY: &[u8] = b"pxxxbbb";

fn xor_encrypt_decrypt(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

fn encrypt_private_key(plain_key: &str) -> String {
    let encrypted = xor_encrypt_decrypt(plain_key.as_bytes(), XOR_KEY);
    BASE64.encode(&encrypted)
}

fn decrypt_private_key(encrypted_key: &str) -> Result<String> {
    let decoded = BASE64
        .decode(encrypted_key)
        .map_err(|e| anyhow::anyhow!("Failed to decode base64: {}", e))?;
    let decrypted = xor_encrypt_decrypt(&decoded, XOR_KEY);
    String::from_utf8(decrypted)
        .map_err(|e| anyhow::anyhow!("Failed to decode decrypted key as UTF-8: {}", e))
}

#[derive(Deserialize)]
struct AppConfig {
    private_key: String,
    // buy_sol_amount: f64,
    // buy_tips_sol_amount: f64,
    // sell_tips_sol_amount: f64,
    // slippage_basis_points: u64,
    nonce_account: String,
    // buy_cu_limit: u32,
    // sell_cu_limit: u32,
    // buy_cu_price: u64,
    // sell_cu_price: u64,
}

impl AppConfig {
    fn load(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file '{}': {}", path, e))?;
        let config: AppConfig = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse config file '{}': {}", path, e))?;
        Ok(config)
    }
}

static CONSOLE_LOGGER: ConsoleLogger = ConsoleLogger;

struct ConsoleLogger;

impl log::Log for ConsoleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            println!(
                "[{}] {}",
                Local::now().format("%Y-%m-%d %H:%M:%S:%3f"),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct SellArgs {
    /// 1: 合约; 2. 卖出百分比(10%:0.1)
    #[clap(env = "PUMP_MINT")]
    pub pump_mint: String,

    #[clap(env = "SELL_PERCENT")]
    pub sell_percent: String,
}

#[derive(Parser)]
#[clap(version, about, long_about = None)]
#[clap(propagate_version = true)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run {},
    Sell { mint: String },
    SellAmm { mint: String },
    QueryTokens {},
    SellAll {},
    Nonce {},
    CashBack {},
    CashBackQuery { address: String },
    Test {},
    TestAmm {},
    TestDevHolding { mint: String, creator: String },
}

#[tokio::main(flavor = "multi_thread", worker_threads = 64)]
async fn main() -> Result<()> {
    log::set_logger(&CONSOLE_LOGGER).map(|()| log::set_max_level(LevelFilter::Info))?;
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cli = Cli::parse();
    if let Commands::Test {} = &cli.command {
        let _ = run_test(Keypair::new()).await?;
    }

    let config = AppConfig::load("config.toml")?;

    let payer = Keypair::from_base58_string(&decrypt_private_key(&config.private_key)?);
    let nonce_account = Pubkey::from_str(&config.nonce_account)?;

    let buy_sol_amount = sol_to_lamports(0.5);
    let buy_tips_sol_amount = 0.00035;
    let sell_tips_sol_amount = 0.00022;

    let cli = Cli::parse();
    match &cli.command {
        Commands::Run {} => {
            let _ = run(
                payer,
                nonce_account,
                buy_sol_amount,
                buy_tips_sol_amount,
                sell_tips_sol_amount,
            )
            .await?;
        }
        Commands::Sell { mint } => {
            let _ = sell(payer, mint.to_string()).await;
        }
        Commands::SellAmm { mint } => {
            let _ = sell_amm(payer, mint.to_string()).await;
        }
        Commands::QueryTokens {} => {
            let _ = query_all_tokens(&payer).await;
        }
        Commands::SellAll {} => {
            let _ = sell_all_tokens(payer).await;
        }
        Commands::Nonce {} => {
            let _ = create_nonce(payer).await?;
        }
        Commands::CashBack {} => {
            let _ = cash_back(payer).await?;
        }
        Commands::CashBackQuery { address } => {
            let _ = cash_back_query(&address).await?;
        }
        Commands::Test {} => {
            let _ = run_test(payer).await?;
        }
        Commands::TestAmm {} => {
            let _ = run_test_amm(payer).await?;
        }
        Commands::TestDevHolding { mint, creator } => {
            let _ = test_dev_holding(payer, mint.to_string(), creator.to_string()).await?;
        }
    }

    Ok(())
}

async fn run(
    payer: Keypair,
    nonce_account: Pubkey,
    buy_sol_amount: u64,
    buy_tips_sol_amount: f64,
    sell_tips_sol_amount: f64,
) -> Result<()> {
    // == 钱包设置 ==
    let payer_pubkey = payer.pubkey();
    let slippage_basis_points = 900;
    // RPC 地址
    // let rpc_url = "http://fra.corvus-labs.io:8899".to_string();
    let rpc_url = "http://fr.rpc.gadflynode.com/".to_string();
    // grpc 地址
    // let corvus_grpc_ams = "http://fra.corvus-labs.io:10101".to_string();
    let corvus_grpc_ams = "http://fr.grpc.gadflynode.com:25565/".to_string();

    let commitment = CommitmentConfig::processed();
    // 可以配置多个SWQOS服务
    let swqos_configs: Vec<SwqosConfig> = vec![
        SwqosConfig::Jito("".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::ZeroSlot("aaaa".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Temporal("bbbbb".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::FlashBlock("cccc".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::BlockRazor("dddd".to_string(), SwqosRegion::Frankfurt, None, None),
        SwqosConfig::Astralane("eeeee".to_string(), SwqosRegion::Frankfurt, None, None),
    ];
    // 创建 TradeConfig 实例
    let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
        .mev_protection(true)
        .use_seed_optimize(false)
        .log_enabled(false)
        .build();
    // 创建 SolanaTrade 客户端
    let client = SolanaTrade::new(Arc::new(payer), trade_config).await;

    // 创建 GasFeeStrategy 实例
    let gas_fee_strategy = GasFeeStrategy::new();
    // 设置全局策略
    gas_fee_strategy.set_global_fee_strategy(
        120000,
        120000,
        2500000,
        1000000,
        buy_tips_sol_amount,
        sell_tips_sol_amount,
    );

    let mut engine: Engine<Event, Action> = Engine::default();

    // buy executor
    let executor = Box::new(x_sol::strategy_2::executor_swap::SwapExecutor::new(
        client.clone(),
        gas_fee_strategy.clone(),
        buy_sol_amount,
        slippage_basis_points,
    ));
    // let executor = Box::new(x_sol::strategy_2::executor_swap_single_exit::SingleExitSwapExecutor::new(
    //     client.clone(),
    //     gas_fee_strategy.clone(),
    //     buy_sol_amount,
    //     slippage_basis_points,
    // ));
    let executor = ExecutorMap::new(executor, |action| match action {
        Action::SubmitAction(bundle) => Some(Action::SubmitAction(bundle)),
        Action::SubmitPumpswapAction(bundle) => Some(Action::SubmitPumpswapAction(bundle)),
        Action::LastBlockHashAndNonceInfoAction(info) => {
            Some(Action::LastBlockHashAndNonceInfoAction(info))
        }
        Action::HeartBeatAction(timestamp) => Some(Action::HeartBeatAction(timestamp)),
    });
    engine.add_executor(Box::new(executor));

    let strategy = Box::new(x_sol::strategy_2::strategy::XStrategyDevSell::new());
    engine.add_strategy(strategy);
    let strategy = Box::new(x_sol::strategy_2::strategy2::XStrategyDevSell::new());
    engine.add_strategy(strategy);

    // grpc self
    let event_collector = Box::new(
        x_sol::collector::pumpfun_event_collector_grpc_new::EventCollector::new(
            "grpc_self_collector".to_string(),
            corvus_grpc_ams.clone(),
            None,
            vec![payer_pubkey.to_string()],
        ),
    );
    let event_collector = CollectorMap::new(event_collector, Event::DexEvent);
    engine.add_collector(Box::new(event_collector));

    // grpc1
    let event_collector = Box::new(
        x_sol::collector::pumpfun_event_collector_grpc_new::EventCollector::new(
            "grpc1_collector".to_string(),
            corvus_grpc_ams.clone(),
            None,
            vec![
                program_ids::PUMPFUN_PROGRAM_ID.to_string(),
                payer_pubkey.to_string(),
            ],
        ),
    );
    let event_collector = CollectorMap::new(event_collector, Event::DexEvent);
    engine.add_collector(Box::new(event_collector));

    // // grpc2
    // let event_collector = Box::new(
    //     x_sol::collector::pumpfun_event_collector_grpc::EventCollector::new(
    //         corvus_grpc_ldn.clone(),
    //         vec![
    //             program_ids::PUMPFUN_PROGRAM_ID.to_string(),
    //         ],
    //     ),
    // );
    // let event_collector = CollectorMap::new(event_collector, Event::DexEvent);
    // engine.add_collector(Box::new(event_collector));

    // get last blockhash and nonce info collector
    let event_collector = Box::new(
        x_sol::collector::lastblockhash_collector::EventCollector::new(
            "last_blockhash_nonce_collector".to_string(),
            client.get_rpc().url(),
            nonce_account,
        ),
    );
    let event_collector = CollectorMap::new(event_collector, Event::LastBlockHashAndNonceInfoEvent);
    engine.add_collector(Box::new(event_collector));

    // heart beat collector
    let event_collector = Box::new(x_sol::collector::heart_beat_collector::EventCollector::new(
        "heart_beat_collector".to_string(),
        1,
    ));
    let event_collector = CollectorMap::new(event_collector, Event::HeartBeatEvent);
    engine.add_collector(Box::new(event_collector));

    // block time collector
    // let event_collector = Box::new(blocktime_collector_grpc::EventCollector::new(
    //     corvus_grpc_ams.clone(),
    //     vec![program_ids::PUMPFUN_PROGRAM_ID.to_string()],
    // ));
    // let event_collector = CollectorMap::new(event_collector, Event::BlockTimeEvent);
    // engine.add_collector(Box::new(event_collector));

    // Telegram bot (独立 task，不影响 Engine 性能)
    let telegram_executor = x_sol::core::executor_telegram::TelegramExecutor::new(
        client.clone(),
        gas_fee_strategy.clone(),
        slippage_basis_points,
        "aaa".to_string(),
        "bbb".to_string(),
    );
    tokio::spawn(async move {
        telegram_executor.start().await;
    });

    // Start engine
    if let Ok(mut set) = engine.run().await {
        while let Some(res) = set.join_next().await {
            log::info!("res: {:?}", res)
        }
    }
    Ok(())
}

async fn sell(payer: Keypair, mint: String) -> Result<()> {
    let mint_pubkey = Pubkey::from_str(&mint)?;
    let nonce_account = Pubkey::from_str("aaa")?;
    // RPC 地址
    let rpc_url = "https://mainnet.helius-rpc.com/?api-key=aaa".to_string();
    let commitment = CommitmentConfig::processed();
    // 可以配置多个SWQOS服务
    let swqos_configs: Vec<SwqosConfig> = vec![
        SwqosConfig::Jito("".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::ZeroSlot("aaaa".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Temporal("bbbbb".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::FlashBlock("cccc".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::BlockRazor("dddd".to_string(), SwqosRegion::Frankfurt, None, None),
        SwqosConfig::Astralane("eeeee".to_string(), SwqosRegion::Frankfurt, None, None),
    ];
    // 创建 TradeConfig 实例
    let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
        .mev_protection(true)
        .use_seed_optimize(false)
        .log_enabled(false)
        .build();
    // 创建 SolanaTrade 客户端
    let client = SolanaTrade::new(Arc::new(payer), trade_config).await;
    // 创建 GasFeeStrategy 实例
    let gas_fee_strategy = GasFeeStrategy::new();
    // 设置全局策略
    gas_fee_strategy.set_global_fee_strategy(110000, 120000, 22222, 11111, 0.0001, 0.0001);
    let slippage_basis_points = Some(500);
    // Sell tokens
    log::info!("Selling tokens from PumpFun...");
    let durable_nonce = fetch_nonce_info(&client.get_rpc(), nonce_account).await;
    log::info!("nonce: {:?}", durable_nonce);
    let rpc = client.get_rpc().clone();
    let payer = client.payer.pubkey();
    log::info!("payer: {:?}", payer);
    let pump_param = PumpFunParams::from_mint_by_rpc(&rpc, &mint_pubkey).await?;
    let account = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &mint_pubkey,
        &pump_param.token_program,
        client.use_seed_optimize,
    );
    log::info!("account: {:?}", account);
    let balance = rpc.get_token_account_balance(&account).await?;
    log::info!("Balance: {:?}", balance);
    let amount_token = balance.amount.parse::<u64>().unwrap();
    log::info!("Selling {} tokens", amount_token);

    let extension_params = DexParamEnum::PumpFun(pump_param);
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::PumpFun,
        output_token_type: TradeTokenType::SOL,
        mint: mint_pubkey,
        input_token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: None,
        with_tip: true,
        extension_params,
        address_lookup_table_account: None,
        wait_tx_confirmed: true,
        create_output_token_ata: false,
        close_output_token_ata: false,
        durable_nonce,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy.clone(),
        close_mint_token_ata: true,
        simulate: false,
        grpc_recv_us: None,
    };
    client.sell(sell_params).await?;
    Ok(())
}

async fn sell_amm(payer: Keypair, mint: String) -> Result<()> {
    let mint_pubkey = Pubkey::from_str(&mint)?;
    let nonce_account = Pubkey::from_str("aaa")?;
    let rpc_url = "https://mainnet.helius-rpc.com/?api-key=aaaa".to_string();
    let commitment = CommitmentConfig::processed();
    let swqos_configs: Vec<SwqosConfig> = vec![
        SwqosConfig::Jito("".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::ZeroSlot("aaaa".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Temporal("bbbbb".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::FlashBlock("cccc".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::BlockRazor("dddd".to_string(), SwqosRegion::Frankfurt, None, None),
        SwqosConfig::Astralane("eeeee".to_string(), SwqosRegion::Frankfurt, None, None),
    ];
    let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
        .mev_protection(true)
        .use_seed_optimize(false)
        .log_enabled(true)
        .build();
    let client = SolanaTrade::new(Arc::new(payer), trade_config).await;

    let gas_fee_strategy = GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(110000, 120000, 22222, 11111, 0.0001, 0.0001);
    let slippage_basis_points = Some(500);

    log::info!("Selling tokens from PumpSwap...");
    let durable_nonce = fetch_nonce_info(&client.get_rpc(), nonce_account).await;
    log::info!("nonce: {:?}", durable_nonce);

    let rpc = client.get_rpc().clone();
    let payer = client.payer.pubkey();
    log::info!("payer: {:?}", payer);

    let pump_swap_param = PumpSwapParams::from_mint_by_rpc(&rpc, &mint_pubkey).await?;
    let token_program = if pump_swap_param.base_mint == mint_pubkey {
        pump_swap_param.base_token_program
    } else {
        pump_swap_param.quote_token_program
    };
    let account = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &mint_pubkey,
        &token_program,
        client.use_seed_optimize,
    );
    log::info!("account: {:?}", account);

    let balance = rpc.get_token_account_balance(&account).await?;
    log::info!("Balance: {:?}", balance);
    let amount_token = balance.amount.parse::<u64>().unwrap();
    log::info!("Selling {} tokens", amount_token);

    let output_token_type = if pump_swap_param.base_mint
        == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
        || pump_swap_param.quote_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
    {
        TradeTokenType::SOL
    } else {
        TradeTokenType::USDC
    };
    let extension_params = DexParamEnum::PumpSwap(pump_swap_param);
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::PumpSwap,
        output_token_type,
        mint: mint_pubkey,
        input_token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: None,
        with_tip: true,
        extension_params,
        address_lookup_table_account: None,
        wait_tx_confirmed: true,
        create_output_token_ata: false,
        close_output_token_ata: false,
        durable_nonce,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy.clone(),
        close_mint_token_ata: true,
        simulate: false,
        grpc_recv_us: None,
    };
    client.sell(sell_params).await?;
    Ok(())
}

async fn create_nonce(payer: Keypair) -> Result<()> {
    let rpc_url =
        "https://mainnet.helius-rpc.com/?api-key=ccccc".to_string();
    let commitment = CommitmentConfig::processed();
    // 可以配置多个SWQOS服务
    let swqos_configs: Vec<SwqosConfig> = vec![SwqosConfig::Default(rpc_url.clone())];
    // 创建 TradeConfig 实例
    let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
        .mev_protection(true)
        .use_seed_optimize(false)
        .log_enabled(false)
        .build();
    // 创建 SolanaTrade 客户端
    let client = SolanaTrade::new(Arc::new(payer), trade_config).await;

    let nonce_account = Keypair::new();
    let nonce_rent = client
        .get_rpc()
        .get_minimum_balance_for_rent_exemption(solana_nonce::state::State::size())
        .await?;
    let instr = solana_program::example_mocks::solana_sdk::system_instruction::create_nonce_account(
        &client.get_payer_pubkey(),
        &nonce_account.pubkey(),
        &client.get_payer_pubkey(), // Make the fee payer the nonce account authority
        nonce_rent,
    );
    let mut tx = solana_sdk::transaction::Transaction::new_with_payer(
        &instr,
        Some(&client.get_payer_pubkey()),
    );
    let blockhash = client.get_rpc().get_latest_blockhash().await?;
    tx.try_sign(&[&nonce_account, &client.get_payer()], blockhash)?;

    let signatture = client.get_rpc().send_and_confirm_transaction(&tx).await?;
    log::info!(
        "key: {}, nonce_account: {}, signatture: {}",
        nonce_account.to_base58_string(),
        nonce_account.pubkey(),
        signatture
    );

    Ok(())
}

async fn cash_back(payer: Keypair) -> Result<()> {
    let rpc_url =
        "https://mainnet.helius-rpc.com/?api-key=dddd4".to_string();
    let commitment = CommitmentConfig::processed();
    // 可以配置多个SWQOS服务
    let swqos_configs: Vec<SwqosConfig> = vec![SwqosConfig::Default(rpc_url.clone())];
    // 创建 TradeConfig 实例
    let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
        .mev_protection(true)
        .use_seed_optimize(false)
        .log_enabled(false)
        .build();
    // 创建 SolanaTrade 客户端
    let client = SolanaTrade::new(Arc::new(payer), trade_config).await;

    let signature = client.claim_cashback_pumpfun().await?;
    log::info!("signature: {}", signature);

    Ok(())
}
