use crate::bindings::ExactlyEvents;
use crate::config::Config;
use crate::credit_service::{Account, AggregatorProxy, Auditor, InterestRateModel, Market};
use crate::fixed_point_math::FixedPointMath;
use ethers::abi::{Tokenizable, Tokenize};
use ethers::prelude::signer::SignerMiddlewareError;
use ethers::prelude::{
    abi::{Abi, RawLog},
    types::Filter,
    Address, EthLogDecode, LogMeta, Middleware, Multicall, Signer, SignerMiddleware, U256, U64,
};
use ethers::prelude::{Log, PubsubClient};
use eyre::Result;
use serde::Deserialize;
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::str::FromStr;
use std::sync::Arc;
use std::{collections::HashMap, time::Duration};
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::Mutex;
use tokio::time;
use tokio_stream::StreamExt;

use super::{ExactlyOracle, Liquidator, MarketAccount, Previewer};

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
enum ContractKeyKind {
    Market,
    PriceFeed,
    Oracle,
    Auditor,
}

enum LogIterating {
    NextLog,
    UpdateFilters,
}

#[derive(Debug)]
enum TaskActivity {
    StartCheckLiquidation,
    StopCheckLiquidation,
    BlockUpdated(Option<U64>),
}

#[derive(Default, Debug)]
struct Repay {
    pub price: U256,
    pub decimals: u8,
    pub seize_available: U256,
    pub seizable_collateral: (U256, Option<Address>),
    pub market_to_liquidate: (U256, Option<Address>),
    pub total_collateral: U256,
    pub adjusted_collateral: U256,
    pub total_debt: U256,
    pub adjusted_debt: U256,
}

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
struct ContractKey {
    address: Address,
    kind: ContractKeyKind,
}

macro_rules! compare {
    ($label:expr, $account:expr, $market:expr, $ref:expr, $val:expr) => {
        if $ref != $val {
            println!("{}@{}.{}", $account, $market, $label);
            println!("reference: {:#?}", $ref);
            println!("    value: {:#?}", $val);
            false
        } else {
            true
        }
    };
}

#[derive(Default)]
struct LiquidationIncentive {
    pub liquidator: u128,
    pub lenders: u128,
}

// #[derive(Debug)]
pub struct CreditService<M, S> {
    client: Arc<SignerMiddleware<M, S>>,
    last_sync: (U64, i128, i128),
    previewer: Previewer<SignerMiddleware<M, S>>,
    auditor: Auditor<SignerMiddleware<M, S>>,
    oracle: ExactlyOracle<SignerMiddleware<M, S>>,
    liquidator: Liquidator<SignerMiddleware<M, S>>,
    markets: HashMap<Address, Market<M, S>>,
    sp_fee_rate: U256,
    accounts: HashMap<Address, Account>,
    contracts_to_listen: HashMap<ContractKey, Address>,
    comparison_enabled: bool,
    liquidation_incentive: LiquidationIncentive,
    token_pairs: HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>,
    tokens: HashSet<Address>,
}

impl<M: 'static + Middleware, S: 'static + Signer> std::fmt::Debug for CreditService<M, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreditService").finish()
    }
}

impl<M: 'static + Middleware, S: 'static + Signer> CreditService<M, S> {
    async fn get_contracts(
        client: Arc<SignerMiddleware<M, S>>,
        config: &Config,
    ) -> (
        u64,
        Auditor<SignerMiddleware<M, S>>,
        Previewer<SignerMiddleware<M, S>>,
        ExactlyOracle<SignerMiddleware<M, S>>,
        Liquidator<SignerMiddleware<M, S>>,
    ) {
        let (auditor_address, _, deployed_block) = CreditService::<M, S>::parse_abi(&format!(
            "node_modules/@exactly-protocol/protocol/deployments/{}/Auditor.json",
            config.chain_id_name
        ));

        let (previewer_address, _, _) = CreditService::<M, S>::parse_abi(&format!(
            "node_modules/@exactly-protocol/protocol/deployments/{}/Previewer.json",
            config.chain_id_name
        ));

        let (liquidator_address, _, _) =
            CreditService::<M, S>::parse_abi("deployments/rinkeby/Liquidator.json");

        let auditor = Auditor::new(auditor_address, Arc::clone(&client));
        let previewer = Previewer::new(previewer_address, Arc::clone(&client));
        let oracle = ExactlyOracle::new(Address::zero(), Arc::clone(&client));
        let liquidator = Liquidator::new(liquidator_address, Arc::clone(&client));
        (deployed_block, auditor, previewer, oracle, liquidator)
    }

    pub async fn new(
        client: Arc<SignerMiddleware<M, S>>,
        config: &Config,
    ) -> Result<CreditService<M, S>> {
        let (deployed_block, auditor, previewer, oracle, liquidator) =
            Self::get_contracts(Arc::clone(&client), &config).await;

        let auditor_markets = auditor.all_markets().call().await?;
        let mut markets = HashMap::<Address, Market<M, S>>::new();
        for market in auditor_markets {
            println!("Adding market {:?}", market);
            markets
                .entry(market)
                .or_insert_with_key(|key| Market::new(*key, &client));
        }
        let mut contracts_to_listen = HashMap::new();

        contracts_to_listen.insert(
            ContractKey {
                address: (*auditor).address(),
                kind: ContractKeyKind::Auditor,
            },
            (*auditor).address(),
        );

        let (token_pairs, tokens) = parse_token_pairs(&config.token_pairs);

        Ok(CreditService {
            client: Arc::clone(&client),
            last_sync: (U64::from(deployed_block), -1, -1),
            auditor,
            previewer,
            oracle,
            liquidator,
            markets,
            sp_fee_rate: U256::zero(),
            accounts: HashMap::new(),
            contracts_to_listen,
            comparison_enabled: config.comparison_enabled,
            liquidation_incentive: Default::default(),
            token_pairs,
            tokens,
        })
    }

    pub async fn update_client(&mut self, client: Arc<SignerMiddleware<M, S>>, config: &Config) {
        let (_, auditor, previewer, oracle, liquidator) =
            Self::get_contracts(Arc::clone(&client), config).await;
        self.client = Arc::clone(&client);
        self.auditor = auditor;
        self.previewer = previewer;
        self.oracle = oracle;
        self.liquidator = liquidator;
        for market in self.markets.values_mut() {
            let address = market.contract.address();
            market.contract =
                crate::credit_service::market_mod::Market::new(address, Arc::clone(&client));
        }
    }

    pub async fn launch(self) -> Result<Self, Self>
    where
        <M as Middleware>::Provider: PubsubClient,
    {
        const PAGE_SIZE: u64 = 5000;
        let service = Arc::new(Mutex::new(self));
        let (debounce_tx, mut debounce_rx) = mpsc::channel(10);
        let me = Arc::clone(&service);
        let a = tokio::spawn(async move {
            let mut block_number = None;
            let mut check_liquidations = false;
            let mut liquidation_checked = false;
            loop {
                let d = Duration::from_millis(2_000);
                match time::timeout(d, debounce_rx.recv()).await {
                    Ok(Some(activity)) => match activity {
                        TaskActivity::StartCheckLiquidation => check_liquidations = true,
                        TaskActivity::StopCheckLiquidation => check_liquidations = false,
                        TaskActivity::BlockUpdated(block) => {
                            block_number = block;
                            println!("### just received log");
                            liquidation_checked = false;
                        }
                    },
                    Ok(None) => {
                        println!("### end of stream");
                    }
                    Err(_) => {
                        println!("### {:?}ms since network activity", d.as_millis());
                        if check_liquidations && !liquidation_checked {
                            if let Some(block) = block_number {
                                println!("### checking liquidations");
                                let lock = me.lock().await.check_liquidations(block).await;
                                liquidation_checked = true;
                                drop(lock);
                            }
                        }
                    }
                }
            }
        });
        println!(
            "=================test===================== {:#?}",
            service.lock().await.last_sync
        );
        let mut first_block = service.lock().await.last_sync.0;
        let mut last_block = first_block + PAGE_SIZE;
        let mut latest_block = U64::zero();
        let mut getting_logs = true;
        'filter: loop {
            let client;
            let result = {
                let service_unlocked = service.lock().await;
                client = Arc::clone(&service_unlocked.client);
                if latest_block.is_zero() {
                    latest_block = client
                        .get_block_number()
                        .await
                        .unwrap_or(service_unlocked.last_sync.0);
                }

                let mut filter = Filter::new().from_block(first_block).address(
                    service_unlocked
                        .contracts_to_listen
                        .values()
                        .cloned()
                        .collect::<Vec<Address>>(),
                );
                println!(">>> Start block: {}", first_block);
                if getting_logs {
                    println!(">>> Last block: {}", last_block);
                    filter = filter.to_block(last_block);
                    let result = client.get_logs(&filter).await;
                    if let Ok(logs) = result {
                        (None, Some(logs))
                    } else {
                        a.abort();
                        println!(">>> Error getting logs: {:?}", result);
                        break 'filter;
                    }
                } else {
                    println!(">>> Getting stream");
                    (Some(client.subscribe_logs(&filter).await), None)
                }
            };
            match result {
                (None, Some(logs)) => {
                    _ = debounce_tx.send(TaskActivity::StopCheckLiquidation).await;
                    let mut me = service.lock().await;
                    for log in logs {
                        println!("handle get log");
                        let status = me.handle_log(log, &debounce_tx).await;
                        println!("handled log");
                        match status {
                            Ok(result) => {
                                if let LogIterating::UpdateFilters = result {
                                    println!(">>> update filter, keep current first block");
                                    first_block = me.last_sync.0;
                                    continue 'filter;
                                }
                            }
                            Err(e) => {
                                println!("Error {:#?}", e);
                                break 'filter;
                            }
                        };
                    }
                    if last_block >= latest_block {
                        println!(">>> reach the end");
                        first_block = last_block + 1u64;
                        getting_logs = false;
                    } else {
                        // println!("comparing every {} blocks", PAGE_SIZE);
                        // let to_timestamp = me
                        //     .client
                        //     .provider()
                        //     .get_block(last_block)
                        //     .await
                        //     .unwrap()
                        //     .unwrap()
                        //     .timestamp;
                        // me.compare_accounts(U64::from(last_block), to_timestamp)
                        //     .await
                        //     .unwrap();

                        first_block = last_block + 1u64;
                        last_block = if first_block + PAGE_SIZE > latest_block {
                            println!(">>> go to final block");
                            latest_block
                        } else {
                            println!(">>> next page");
                            last_block + PAGE_SIZE
                        };
                    }
                }
                (Some(stream), None) => {
                    _ = debounce_tx.send(TaskActivity::StartCheckLiquidation).await;
                    println!(">>> checking stream");
                    match stream {
                        Ok(mut stream) => {
                            // let mut last_block = U64::zero();
                            println!(">>> waiting next block");
                            while let Some(log) = stream.next().await {
                                // if let Some(block_number) = log.block_number {
                                //     if block_number != last_block {
                                //         if last_block > U64::zero() {
                                //             let to_timestamp = service
                                //                 .lock()
                                //                 .await
                                //                 .client
                                //                 .provider()
                                //                 .get_block(last_block)
                                //                 .await
                                //                 .unwrap()
                                //                 .unwrap()
                                //                 .timestamp;
                                //             service
                                //                 .lock()
                                //                 .await
                                //                 .compare_accounts(U64::from(last_block), to_timestamp)
                                //                 .await
                                //                 .unwrap();
                                //         }
                                //         last_block = block_number;
                                //     }
                                // }
                                println!(">>> lock service");
                                let mut me = service.lock().await;
                                println!(">>> service locked");
                                let status = me.handle_log(log, &debounce_tx).await;
                                match status {
                                    Ok(result) => {
                                        if let LogIterating::UpdateFilters = result {
                                            first_block = me.last_sync.0;
                                            continue 'filter;
                                        }
                                    }
                                    Err(e) => {
                                        println!("Error {:#?}", e);
                                        break 'filter;
                                    }
                                };
                            }
                        }
                        e => {
                            if let Err(error) = e {
                                if let SignerMiddlewareError::MiddlewareError(m) = error {
                                    println!("error to subscribe {:#?}", m);
                                    continue;
                                }
                            }
                            println!("error to subscribe");
                            break;
                        }
                    }
                }
                (_, _) => {}
            };
        }
        a.abort();
        // if error {
        //     return Err(Arc::try_unwrap(service).unwrap().into_inner());
        // }
        Ok(Arc::try_unwrap(service).unwrap().into_inner())
    }

    async fn update_prices(&mut self, block: U64) -> Result<()> {
        println!("\nUpdating prices for block {}:\n", block);
        let mut multicall =
            Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None).await?;
        let mut update = false;
        for (market, _) in self.markets.iter().filter(|(_, v)| v.listed) {
            update = true;
            multicall.add_call(self.oracle.asset_price(*market));
        }
        if !update {
            return Ok(());
        }
        println!("call multicall for updating prices for block {}", block);
        let result = multicall.block(block).call_raw().await?;
        for (i, market) in self.markets.values_mut().filter(|m| m.listed).enumerate() {
            market.oracle_price = result[i].clone().into_uint().unwrap();
            println!("new price: {:#?}", market.oracle_price);
        }

        Ok(())
    }

    // handle a new received log
    async fn handle_log(
        &mut self,
        log: Log,
        sender: &Sender<TaskActivity>,
    ) -> Result<LogIterating> {
        let meta = LogMeta::from(&log);
        let result = ExactlyEvents::decode_log(&RawLog {
            topics: log.topics,
            data: log.data.to_vec(),
        });
        if let Err(_) = &result {
            println!("{:?}", meta);
        }
        let event = result?;

        println!(
            "---->     Contract {:?} - {}",
            meta.address, meta.block_number
        );
        // if meta.block_number >= 11276623u64.into() {
        //     println!("------- event with block number 11276623");
        //     sender.send(TaskActivity::StartCheckLiquidation).await?;
        //     std::thread::sleep(Duration::from_secs(2));
        //     return Ok(LogIterating::UpdateFilters);
        // }
        if (
            meta.block_number,
            meta.transaction_index.as_u64() as i128,
            meta.log_index.as_u128() as i128,
        ) <= self.last_sync
        {
            return Ok(LogIterating::NextLog);
        }
        self.last_sync = (
            meta.block_number,
            meta.transaction_index.as_u64() as i128,
            meta.log_index.as_u128() as i128,
        );
        println!("{:?}, {:?}", event, meta);
        match event {
            ExactlyEvents::MaxFuturePoolsSetFilter(data) => {
                self.markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| Market::new(*key, &self.client))
                    .max_future_pools = data.max_future_pools.as_u32() as u8;
            }

            ExactlyEvents::OracleSetFilter(data) => {
                self.oracle = ExactlyOracle::new(data.oracle, self.client.clone());
                self.update_prices(meta.block_number).await?;
                self.contracts_to_listen
                    .entry(ContractKey {
                        address: (*self.auditor).address(),
                        kind: ContractKeyKind::Oracle,
                    })
                    .or_insert(data.oracle);
                self.last_sync = (
                    meta.block_number,
                    meta.transaction_index.as_u64() as i128,
                    meta.log_index.as_u128() as i128,
                );
                sender
                    .send(TaskActivity::BlockUpdated(log.block_number))
                    .await?;
                return Ok(LogIterating::UpdateFilters);
            }

            ExactlyEvents::EarningsAccumulatorSmoothFactorSetFilter(data) => {
                self.markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| Market::new(*key, &self.client))
                    .earnings_accumulator_smooth_factor = data.earnings_accumulator_smooth_factor;
            }

            ExactlyEvents::MarketListedFilter(data) => {
                let mut market = self
                    .markets
                    .entry(data.market)
                    .or_insert_with_key(|key| Market::new(*key, &self.client));
                market.decimals = data.decimals;
                market.smart_pool_fee_rate = self.sp_fee_rate;
                market.listed = true;
                let mut multicall =
                    Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None)
                        .await
                        .unwrap();
                multicall.add_call(market.contract.max_future_pools());
                multicall.add_call(market.contract.earnings_accumulator_smooth_factor());
                multicall.add_call(market.contract.interest_rate_model());
                multicall.add_call(market.contract.penalty_rate());
                multicall.add_call(market.contract.treasury_fee_rate());
                multicall.add_call(market.contract.asset());
                multicall = multicall.block(meta.block_number);
                println!("Call multicall for market {}", market.contract.address());
                let (
                    max_future_pools,
                    earnings_accumulator_smooth_factor,
                    interest_rate_model,
                    penalty_rate,
                    treasury_fee_rate,
                    asset,
                ) = multicall.call().await?;
                market.max_future_pools = max_future_pools;
                market.earnings_accumulator_smooth_factor = earnings_accumulator_smooth_factor;
                market.interest_rate_model = interest_rate_model;
                market.penalty_rate = penalty_rate;
                market.treasury_fee_rate = treasury_fee_rate;
                market.asset = asset;

                let irm =
                    InterestRateModel::new(market.interest_rate_model, Arc::clone(&self.client));
                multicall.clear_calls();
                multicall.add_call(irm.floating_curve_a());
                multicall.add_call(irm.floating_curve_b());
                multicall.add_call(irm.floating_max_utilization());
                println!("Call multicall for interest rate model {}", irm.address());
                let result = multicall.block(meta.block_number).call().await;
                if let Ok((floating_a, floating_b, floating_max_utilization)) = result {
                    println!("success getting interest rate model data");
                    market.floating_a = floating_a;
                    market.floating_b = floating_b;
                    market.floating_max_utilization = floating_max_utilization;
                } else {
                    println!("error getting interest rate model data");
                }
                self.contracts_to_listen.insert(
                    ContractKey {
                        address: data.market,
                        kind: ContractKeyKind::Market,
                    },
                    data.market,
                );
                market.approve_asset(&self.client).await?;
                self.update_prices(meta.block_number).await?;
                self.last_sync = (
                    meta.block_number,
                    meta.transaction_index.as_u64() as i128,
                    meta.log_index.as_u128() as i128,
                );
                return Ok(LogIterating::UpdateFilters);
            }

            ExactlyEvents::InterestRateModelSetFilter(data) => {
                let mut market = self.markets.get_mut(&meta.address).unwrap();
                market.interest_rate_model = data.interest_rate_model;
                let irm =
                    InterestRateModel::new(market.interest_rate_model, Arc::clone(&self.client));
                let mut multicall =
                    Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None)
                        .await
                        .unwrap();

                multicall.add_call(irm.floating_curve_a());
                multicall.add_call(irm.floating_curve_b());
                multicall.add_call(irm.floating_max_utilization());
                let result = multicall.block(meta.block_number).call().await;
                if let Ok((floating_a, floating_b, floating_max_utilization)) = result {
                    println!("success getting interest rate model data");
                    market.floating_a = floating_a;
                    market.floating_b = floating_b;
                    market.floating_max_utilization = floating_max_utilization;
                } else {
                    println!("error getting interest rate model data");
                }
                self.last_sync = (
                    meta.block_number,
                    meta.transaction_index.as_u64() as i128,
                    meta.log_index.as_u128() as i128,
                );
                return Ok(LogIterating::UpdateFilters);
            }

            ExactlyEvents::TransferFilter(data) => {
                if data.to != Address::zero() {
                    self.accounts
                        .entry(data.to)
                        .or_insert_with_key(|key| Account::new(*key, &self.markets))
                        .positions
                        .entry(meta.address)
                        .or_default()
                        .floating_deposit_shares += data.amount;
                }
                if data.from != Address::zero() {
                    self.accounts
                        .get_mut(&data.from)
                        .unwrap()
                        .positions
                        .get_mut(&meta.address)
                        .unwrap()
                        .floating_deposit_shares -= data.amount;
                }
            }
            ExactlyEvents::BorrowFilter(data) => {
                self.accounts
                    .entry(data.borrower)
                    .or_insert_with_key(|key| Account::new(*key, &self.markets))
                    .positions
                    .entry(meta.address)
                    .or_default()
                    .floating_borrow_shares += data.shares;
            }
            ExactlyEvents::RepayFilter(data) => {
                self.accounts
                    .entry(data.borrower)
                    .or_insert_with_key(|key| Account::new(*key, &self.markets))
                    .positions
                    .entry(meta.address)
                    .or_default()
                    .floating_borrow_shares -= data.shares;
            }
            ExactlyEvents::DepositAtMaturityFilter(data) => {
                self.accounts
                    .entry(data.owner)
                    .or_insert_with(|| Account::new(data.owner, &self.markets))
                    .deposit_at_maturity(&data, &meta.address);
            }
            ExactlyEvents::WithdrawAtMaturityFilter(data) => {
                self.accounts
                    .entry(data.owner)
                    .or_insert_with(|| Account::new(data.owner, &self.markets))
                    .withdraw_at_maturity(data, &meta.address);
            }
            ExactlyEvents::BorrowAtMaturityFilter(data) => {
                self.accounts
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.markets))
                    .borrow_at_maturity(&data, &meta.address);
            }
            ExactlyEvents::RepayAtMaturityFilter(data) => {
                self.accounts
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.markets))
                    .repay_at_maturity(&data, &meta.address);
            }
            ExactlyEvents::LiquidateFilter(data) => {
                self.accounts
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.markets))
                    .liquidate_borrow(data, &meta.address);
            }
            ExactlyEvents::SeizeFilter(data) => {
                self.accounts
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.markets))
                    .asset_seized(data, &meta.address);
            }

            ExactlyEvents::AdjustFactorSetFilter(data) => {
                self.markets
                    .entry(data.market)
                    .or_insert_with_key(|key| Market::new(*key, &self.client))
                    .adjust_factor = data.adjust_factor;
            }

            ExactlyEvents::PenaltyRateSetFilter(data) => {
                self.markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| Market::new(*key, &self.client))
                    .penalty_rate = data.penalty_rate;
            }

            ExactlyEvents::MarketEnteredFilter(data) => {
                self.accounts
                    .entry(data.account)
                    .or_insert_with(|| Account::new(data.account, &self.markets))
                    .set_collateral(&data.market);
            }

            ExactlyEvents::MarketExitedFilter(data) => {
                self.accounts
                    .entry(data.account)
                    .or_insert_with(|| Account::new(data.account, &self.markets))
                    .unset_collateral(&data.market);
            }

            ExactlyEvents::BackupFeeRateSetFilter(data) => {
                self.sp_fee_rate = data.backup_fee_rate;
                for market in self.markets.values_mut() {
                    market.smart_pool_fee_rate = data.backup_fee_rate;
                }
            }

            ExactlyEvents::PriceFeedSetFilter(data) => {
                let price_feed = AggregatorProxy::new(data.source, Arc::clone(&self.client));
                let aggregator = price_feed.aggregator().call().await.unwrap();
                self.contracts_to_listen
                    .entry(ContractKey {
                        address: data.market,
                        kind: ContractKeyKind::PriceFeed,
                    })
                    .or_insert(aggregator);
                self.markets
                    .entry(data.market)
                    .or_insert_with_key(|key| Market::new(*key, &self.client))
                    .price_feed = aggregator;
                self.update_prices(meta.block_number).await?;
                self.last_sync = (
                    meta.block_number,
                    meta.transaction_index.as_u64() as i128,
                    meta.log_index.as_u128() as i128,
                );
                sender
                    .send(TaskActivity::BlockUpdated(log.block_number))
                    .await?;
                return Ok(LogIterating::UpdateFilters);
            }

            ExactlyEvents::AnswerUpdatedFilter(data) => {
                let market = *self
                    .markets
                    .iter()
                    .find_map(|(address, market)| {
                        if market.price_feed == meta.address {
                            return Some(address);
                        }
                        None
                    })
                    .unwrap();
                println!("=data.current {:#?}", data.current);
                println!("=data.current {:#?}", data.current.into_raw());
                println!(
                    "=data.current {:#?}",
                    data.current.into_raw() * U256::exp10(10)
                );
                self.markets
                    .entry(market)
                    .or_insert_with_key(|key| Market::new(*key, &self.client))
                    .oracle_price = data.current.into_raw() * U256::exp10(10);
            }

            ExactlyEvents::MarketUpdateFilter(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address, &self.client));
                market.floating_deposit_shares = data.floating_deposit_shares;
                market.floating_assets = data.floating_assets;
                market.earnings_accumulator = data.earnings_accumulator;
                market.floating_debt = data.floating_debt;
                market.floating_borrow_shares = data.floating_borrow_shares;
            }

            ExactlyEvents::FloatingDebtUpdateFilter(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address, &self.client));
                market.floating_utilization = data.utilization;
                market.last_floating_debt_update = data.timestamp;
            }

            ExactlyEvents::AccumulatorAccrualFilter(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address, &self.client));
                market.last_accumulator_accrual = data.timestamp;
            }

            ExactlyEvents::FixedEarningsUpdateFilter(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address, &self.client));

                let pool = market.fixed_pools.entry(data.maturity).or_default();
                pool.last_accrual = data.timestamp;
                pool.unassigned_earnings = data.unassigned_earnings;
            }

            ExactlyEvents::TreasurySetFilter(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address, &self.client));
                market.treasury_fee_rate = data.treasury_fee_rate;
            }

            ExactlyEvents::LiquidationIncentiveSetFilter(data) => {
                self.liquidation_incentive = LiquidationIncentive {
                    liquidator: data.liquidation_incentive.0,
                    lenders: data.liquidation_incentive.1,
                }
            }

            _ => {
                println!("Event not handled - {:?}", event);
            }
        }
        println!("send 0");
        sender
            .send(TaskActivity::BlockUpdated(log.block_number))
            .await?;
        println!("send 1");
        Ok(LogIterating::NextLog)
    }

    async fn check_liquidations(&self, block_number: U64) -> Result<()> {
        println!("check_liquidations");
        // let previewer_borrowers = self.multicall_previewer(U64::from(&to)).await;
        let to_timestamp = self
            .client
            .provider()
            .get_block(block_number)
            .await?
            .unwrap()
            .timestamp;
        if (*self.oracle).address() != Address::zero() {
            // self.update_prices(block_number).await?;
        }

        if self.comparison_enabled {
            println!("comparison_enabled");
            self.compare_accounts(block_number, to_timestamp).await?;
        } else {
            println!("not comparison_enabled");
        }

        let mut liquidations: HashMap<Address, (Account, Repay)> = HashMap::new();
        let mut liquidations_counter = 0;
        for (address, account) in self.accounts.iter() {
            let hf = Self::compute_hf(&self.markets, account, to_timestamp);
            if let Ok((hf, _, _, repay)) = hf {
                if hf < U256::exp10(18) && repay.adjusted_debt != U256::zero() {
                    liquidations_counter += 1;
                    liquidations.insert(address.clone(), (account.clone(), repay));
                }
            }
        }
        println!(
            "accounts to check for liquidations {:#?}",
            self.accounts.len()
        );
        println!("accounts to liquidate {:#?}", liquidations_counter);
        self.liquidate(&liquidations).await?;
        Ok(())
    }

    async fn liquidate(&self, liquidations: &HashMap<Address, (Account, Repay)>) -> Result<()> {
        for (_, (account, repay)) in liquidations {
            println!("Liquidating account {:?}", account);
            // if account.address
            //     != Address::from_str("0x279ab4aafeaad01ec6b0bc07957b387b1502248d").unwrap()
            // {
            //     continue;
            // }
            if let Some(address) = &repay.market_to_liquidate.1 {
                let max_repay_assets =
                    Self::max_repay_assets(repay, &self.liquidation_incentive, U256::MAX);

                let (pool_pair, fee): (Address, u32) = self.get_flash_pair(
                    *address,
                    repay.seizable_collateral.1.unwrap(),
                    max_repay_assets,
                );
                println!("Liquidating on fixed lender {:#?}", address);
                println!(
                    "seizing                    {:#?}",
                    repay.seizable_collateral.1.unwrap()
                );
                println!("flash pair                  {:#?}", pool_pair);
                println!("fee                         {:#?}", fee);
                println!("max_repay_assets            {:#?}", max_repay_assets);

                // liquidate using liquidator contract
                let func = self
                    .liquidator
                    .liquidate(
                        *address,
                        repay.seizable_collateral.1.unwrap(),
                        account.address,
                        max_repay_assets,
                        pool_pair,
                        fee,
                    )
                    .gas(6_666_666);

                // liquidate using market liquidate function
                // let func = self.markets[address]
                //     .contract
                //     .liquidate(
                //         account.address,
                //         max_repay_assets,
                //         repay.seizable_collateral.1.unwrap(),
                //     )
                //     .gas(6_666_666);

                let tx = func.send().await;
                println!("tx: {:?}", &tx);
                let tx = tx?;
                println!("waiting receipt");
                let receipt = tx.confirmations(1).await?;
                println!("Liquidation tx {:?}", receipt);
            }
        }
        Ok(())
    }

    async fn multicall_previewer(
        &self,
        block: U64,
    ) -> HashMap<Address, HashMap<Address, MarketAccount>> {
        let mut skip: usize = 0;
        let batch = 100;
        let mut positions = HashMap::<Address, HashMap<Address, MarketAccount>>::new();

        while skip < self.accounts.len() {
            let mut multicall =
                Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None)
                    .await
                    .unwrap();
            let mut inputs: Vec<Address> = Vec::new();
            for account in self.accounts.keys().skip(skip).take(batch) {
                inputs.push(*account);

                multicall.add_call(self.previewer.exactly(*account));
            }
            let responses = multicall.block(block).call_raw().await;
            if let Err(_) = responses {
                println!("old previewer, no comparison will be made");
                return positions;
            }
            let responses = responses.unwrap();

            let mut accounts_updated = inputs.iter();
            for response in responses {
                let payload: Vec<MarketAccount> = Vec::from_token(response).unwrap();
                let account = accounts_updated
                    .next()
                    .expect("Number of self.accounts and responses doesn't match");

                let mut markets = HashMap::new();
                for market_account in payload {
                    markets.insert(market_account.market, market_account);
                }
                positions.insert(*account, markets);
            }

            skip += batch;
        }
        positions
    }

    async fn compare_accounts(&self, block: U64, timestamp: U256) -> Result<()> {
        let previewer_accounts = &self.multicall_previewer(U64::from(&block)).await;
        let accounts = &self.accounts;
        let mut success = true;
        let mut compare_markets = true;
        let mut accounts_with_borrows = Vec::<Address>::new();

        if previewer_accounts.len() == 0 {
            return Ok(());
        };
        println!("previewer on block {}", block);
        for (address, previewer_account) in previewer_accounts {
            let account = &accounts[address];
            success &= compare!(
                "length",
                address,
                "[]",
                previewer_account.len(),
                account.positions.len()
            );
            if compare_markets {
                let mut multicall =
                    Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None)
                        .await?;
                const FIELDS: usize = 9;
                for market in self.markets.values() {
                    multicall.add_call(market.contract.floating_assets());
                    multicall.add_call(market.contract.last_accumulator_accrual());
                    multicall.add_call(market.contract.earnings_accumulator());
                    multicall.add_call(market.contract.earnings_accumulator_smooth_factor());
                    multicall.add_call(market.contract.floating_debt());
                    multicall.add_call(market.contract.total_floating_borrow_shares());
                    multicall.add_call(market.contract.last_floating_debt_update());
                    multicall.add_call(market.contract.floating_utilization());
                    multicall.add_call(market.contract.total_supply());
                }
                let responses = multicall.block(block).call_raw().await?;
                for (i, market) in self.markets.values().enumerate() {
                    let previewer_market = &previewer_account[&market.contract.address()];

                    success &= compare!(
                        "floating_assets",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS].clone().into_uint().unwrap(),
                        market.floating_assets
                    );
                    success &= compare!(
                        "last_accumulator_accrual",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 1].clone().into_uint().unwrap(),
                        market.last_accumulator_accrual
                    );
                    success &= compare!(
                        "earnings_accumulator",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 2].clone().into_uint().unwrap(),
                        market.earnings_accumulator
                    );
                    success &= compare!(
                        "earnings_accumulator_smooth_factor",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 3].clone().into_uint().unwrap(),
                        U256::from(market.earnings_accumulator_smooth_factor)
                    );
                    success &= compare!(
                        "floating_debt",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 4].clone().into_uint().unwrap(),
                        U256::from(market.floating_debt)
                    );
                    success &= compare!(
                        "total_floating_borrow_shares",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 5].clone().into_uint().unwrap(),
                        U256::from(market.floating_borrow_shares)
                    );
                    success &= compare!(
                        "total_floating_borrow_shares",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 5].clone().into_uint().unwrap(),
                        U256::from(market.floating_borrow_shares)
                    );
                    success &= compare!(
                        "last_floating_debt_update",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 6].clone().into_uint().unwrap(),
                        U256::from(market.last_floating_debt_update)
                    );
                    success &= compare!(
                        "floating_utilization",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 7].clone().into_uint().unwrap(),
                        U256::from(market.floating_utilization)
                    );
                    success &= compare!(
                        "total_floating_deposit_shares",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 8].clone().into_uint().unwrap(),
                        U256::from(market.floating_deposit_shares)
                    );
                    success &= compare!(
                        "max_future_pools",
                        "",
                        previewer_market.market,
                        previewer_market.max_future_pools,
                        market.max_future_pools
                    );
                    success &= compare!(
                        "total_floating_deposit_assets",
                        "",
                        previewer_market.market,
                        previewer_market.total_floating_deposit_assets,
                        market.total_assets(timestamp)
                    );
                    success &= compare!(
                        "total_floating_borrow_assets",
                        "",
                        previewer_market.market,
                        previewer_market.total_floating_borrow_assets,
                        market.total_floating_borrow_assets(timestamp)
                    );
                }
            }
            compare_markets = false;
            let mut debt = U256::zero();
            for market_account in previewer_account.values() {
                let account = &self.accounts[address].positions[&market_account.market];
                // let market = &self.markets[&market_account.market];
                success &= compare!(
                    "floating_deposit_shares",
                    address,
                    market_account.market,
                    market_account.floating_deposit_shares,
                    account.floating_deposit_shares
                );
                success &= compare!(
                    "floating_borrow_shares",
                    address,
                    market_account.market,
                    market_account.floating_borrow_shares,
                    account.floating_borrow_shares
                );
                if market_account.floating_borrow_shares > U256::zero() {
                    debt += market_account.floating_borrow_shares;
                }
                for fixed_deposit in &market_account.fixed_deposit_positions {
                    success &= compare!(
                        format!("fixed_deposit_positions[{}]", fixed_deposit.maturity),
                        address,
                        market_account.market,
                        fixed_deposit.position.principal + fixed_deposit.position.fee,
                        account.fixed_deposit_positions[&fixed_deposit.maturity.as_u32()]
                    );
                }
                for fixed_borrow in &market_account.fixed_borrow_positions {
                    success &= compare!(
                        format!("fixed_borrow_positions[{}]", fixed_borrow.maturity),
                        address,
                        market_account.market,
                        fixed_borrow.position.principal + fixed_borrow.position.fee,
                        account.fixed_borrow_positions[&fixed_borrow.maturity.as_u32()]
                    );
                    if (fixed_borrow.position.principal + fixed_borrow.position.fee) > U256::zero()
                    {
                        debt += fixed_borrow.position.principal + fixed_borrow.position.fee;
                    }
                }
            }
            if debt > U256::zero() {
                accounts_with_borrows.push(*address);
            }
        }

        let previewer_incentive = self
            .auditor
            .liquidation_incentive()
            .block(block)
            .call()
            .await
            .unwrap();

        success &= compare!(
            "liquidation_incentive.liquidator",
            "",
            "",
            previewer_incentive.0,
            self.liquidation_incentive.liquidator
        );

        success &= compare!(
            "liquidation_incentive.lenders",
            "",
            "",
            previewer_incentive.1,
            self.liquidation_incentive.lenders
        );

        let mut multicall =
            Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None).await?;
        for account in &accounts_with_borrows {
            multicall.add_call(self.auditor.account_liquidity(
                *account,
                Address::zero(),
                U256::zero(),
            ));
        }
        let timestamp = self
            .client
            .provider()
            .get_block(block)
            .await?
            .unwrap()
            .timestamp;

        let responses = multicall.block(block).call_raw().await?;

        for (i, token) in responses.iter().enumerate() {
            let v = token.clone().into_tokens();
            let (adjusted_collateral, adjusted_debt): (U256, U256) = (
                v[0].clone().into_uint().unwrap(),
                v[1].clone().into_uint().unwrap(),
            );
            if adjusted_debt > U256::zero() {
                let previewer_hf = adjusted_collateral.div_wad_down(adjusted_debt);
                if let Some(account) = self.accounts.get(&accounts_with_borrows[i]) {
                    if let Ok((hf, collateral, debt, _)) =
                        Self::compute_hf(&self.markets, account, timestamp)
                    {
                        success &= compare!(
                            "health_factor",
                            accounts_with_borrows[i],
                            "",
                            previewer_hf,
                            hf
                        );
                        success &= compare!(
                            "collateral",
                            accounts_with_borrows[i],
                            "",
                            adjusted_collateral,
                            collateral
                        );
                        success &=
                            compare!("debt", accounts_with_borrows[i], "", adjusted_debt, debt);
                    }
                }
            } else {
                println!(
                    "Account: {} collateral: {} debt: {}",
                    accounts_with_borrows[i], adjusted_collateral, adjusted_debt
                );
                println!("***************");
                println!("Account: {:#?}", self.accounts[&accounts_with_borrows[i]]);
                println!("***************");
            }
        }
        if !success {
            panic!("compare accounts error");
        }
        Ok(())
    }

    pub fn parse_abi(abi_path: &str) -> (Address, Abi, u64) {
        let file = File::open(abi_path).unwrap();
        let reader = BufReader::new(file);
        let contract: Value = serde_json::from_reader(reader).unwrap();
        let (contract, abi, receipt): (Value, Value, Value) = if let Value::Object(abi) = contract {
            (
                abi["address"].clone(),
                abi["abi"].clone(),
                abi["receipt"].clone(),
            )
        } else {
            panic!("Invalid ABI")
        };
        let contract: Address = if let Value::String(contract) = contract {
            Address::from_str(&contract).unwrap()
        } else {
            panic!("Invalid ABI")
        };
        let abi: Vec<Value> = if let Value::Array(abi) = abi {
            abi
        } else {
            panic!("Invalid ABI")
        };
        let abi: Vec<Value> = abi
            .into_iter()
            .filter(|abi| {
                if let Value::Object(variant) = abi {
                    variant["type"] != "error"
                } else {
                    false
                }
            })
            .collect();
        let abi = Value::Array(abi);
        let abi: Abi = serde_json::from_value(abi.clone()).unwrap();
        let block_number = if let Value::Object(receipt) = receipt {
            if let Value::Number(block_number) = &receipt["blockNumber"] {
                block_number.as_u64().unwrap()
            } else {
                panic!("Invalid ABI")
            }
        } else {
            panic!("Invalid ABI");
        };
        (contract, abi, block_number)
    }

    fn compute_hf(
        markets: &HashMap<Address, Market<M, S>>,
        account: &Account,
        timestamp: U256,
    ) -> Result<(U256, U256, U256, Repay)> {
        let mut total_collateral: U256 = U256::zero();
        let mut adjusted_collateral: U256 = U256::zero();
        let mut total_debt: U256 = U256::zero();
        let mut adjusted_debt: U256 = U256::zero();
        let mut repay = Repay::default();
        for (market_address, position) in account.positions.iter() {
            let market = markets.get(market_address).unwrap();
            if position.is_collateral {
                let current_collateral = position.floating_deposit_assets(market, timestamp);
                let value = current_collateral
                    .mul_div_down(market.oracle_price, U256::exp10(market.decimals as usize));
                total_collateral += value;
                adjusted_collateral += value.mul_wad_down(market.adjust_factor);
                if value > repay.seizable_collateral.0 {
                    repay.seize_available = value;
                    repay.seizable_collateral = (value, Some(*market_address));
                }
            }
            let mut current_debt = U256::zero();

            for (maturity, borrowed) in position.fixed_borrow_positions.iter() {
                current_debt += *borrowed;
                if U256::from(*maturity) < timestamp {
                    current_debt += borrowed.mul_wad_down(
                        (timestamp - U256::from(*maturity)) * U256::from(market.penalty_rate),
                    )
                }
            }
            current_debt += position.floating_borrow_assets(market, timestamp);

            let value =
                current_debt.mul_div_up(market.oracle_price, U256::exp10(market.decimals as usize));
            total_debt += value;
            adjusted_debt += value.div_wad_up(market.adjust_factor);

            if value > repay.market_to_liquidate.0 {
                repay.price = market.oracle_price;
                repay.decimals = market.decimals;
                repay.market_to_liquidate = (value, Some(*market_address));
            }
        }
        repay.total_collateral = total_collateral;
        repay.adjusted_collateral = adjusted_collateral;
        repay.total_debt = total_debt;
        repay.adjusted_debt = adjusted_debt;
        let hf = if adjusted_debt == U256::zero() {
            adjusted_collateral
        } else {
            U256::exp10(18) * adjusted_collateral / adjusted_debt
        };
        if hf < U256::exp10(18) && adjusted_debt != U256::zero() {
            println!("==============");
            println!("Account                {:?}", account.address);
            println!("Total Collateral   USD {:?}", total_collateral);
            println!("Total Debt         USD {:?}", total_debt);
            println!("Health factor {:?}\n", hf);
        }
        Ok((hf, adjusted_collateral, adjusted_debt, repay))
    }

    fn max_repay_assets(
        repay: &Repay,
        liquidation_incentive: &LiquidationIncentive,
        max_liquidator_assets: U256,
    ) -> U256 {
        println!("repay {:?}", repay);
        let target_health = U256::exp10(16usize) * 125u32;
        let adjusted_factor = repay
            .adjusted_collateral
            .div_wad_up(repay.total_collateral)
            .mul_wad_up(repay.total_debt.div_wad_up(repay.adjusted_debt));
        println!("adjusted_factor {:?}", adjusted_factor);
        let close_factor =
            (target_health - repay.adjusted_collateral.div_wad_up(repay.adjusted_debt)).div_wad_up(
                target_health
                    - adjusted_factor.mul_wad_down(
                        U256::exp10(18)
                            + liquidation_incentive.liquidator
                            + liquidation_incentive.lenders,
                    ),
            );
        println!("close_factor {:?}", close_factor);
        U256::min(
            U256::min(
                repay
                    .total_debt
                    .mul_wad_up(U256::min(U256::exp10(18), close_factor)),
                repay.seize_available.div_wad_up(
                    U256::exp10(18)
                        + liquidation_incentive.liquidator
                        + liquidation_incentive.lenders,
                ),
            )
            .mul_div_up(U256::exp10(repay.decimals as usize), repay.price),
            if max_liquidator_assets
                < U256::from_str("115792089237316195423570985008687907853269984665640564039457") //// U256::MAX / WAD
                    .unwrap()
            {
                max_liquidator_assets.div_wad_down(U256::exp10(18) + liquidation_incentive.lenders)
            } else {
                max_liquidator_assets
            },
        )
    }

    fn get_flash_pair(
        &self,
        repay: Address,
        collateral: Address,
        _max_repay_assets: U256,
    ) -> (Address, u32) {
        // shadowing repay and collateral with underlying assets
        let repay = self.markets[&repay].asset;
        let collateral = self.markets[&collateral].asset;

        let mut lowest_fee = u32::MAX;
        let mut pair_contract = Address::zero();
        println!("collateral {:#?}", collateral);
        for token in &self.tokens {
            println!("token      {:#?}", token);
            if *token != collateral {
                if let Some(pair) = self.token_pairs.get(&ordered_addresses(*token, repay)) {
                    println!("pair found");
                    if let Some(rate) = pair.peek() {
                        println!("rate {:?}", rate.0);
                        if rate.0 < lowest_fee {
                            lowest_fee = rate.0;
                            pair_contract = *token;
                        }
                    }
                }
            }
        }
        (pair_contract, lowest_fee)
    }
}

fn ordered_addresses(token0: Address, token1: Address) -> (Address, Address) {
    if token0 < token1 {
        (token0, token1)
    } else {
        (token1, token0)
    }
}

#[derive(Deserialize, Debug)]
pub struct TokenPair {
    pub token0: String,
    pub token1: String,
    pub fee: u32,
}

fn parse_token_pairs(
    token_pairs: &str,
) -> (
    HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>,
    HashSet<Address>,
) {
    let mut tokens = HashSet::new();
    let json_pairs: Vec<(String, String, u32)> = serde_json::from_str(token_pairs).unwrap();
    let mut pairs = HashMap::new();
    for (token0, token1, fee) in json_pairs {
        let token0 = Address::from_str(&token0).unwrap();
        let token1 = Address::from_str(&token1).unwrap();
        tokens.insert(token0);
        tokens.insert(token1);
        pairs
            .entry(ordered_addresses(token0, token1))
            .or_insert(BinaryHeap::new())
            .push(Reverse(fee));
    }
    (pairs, tokens)
}
#[cfg(test)]
mod services_test {

    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[test]
    fn test_parse_token_pairs() {
        let tokens = r#"[[
                            "0x0000000000000000000000000000000000000000", 
                            "0x0000000000000000000000000000000000000001", 
                            3000
                          ],
                          [
                            "0x0000000000000000000000000000000000000000", 
                            "0x0000000000000000000000000000000000000001", 
                            1000
                          ],
                          [
                            "0x0000000000000000000000000000000000000000", 
                            "0x0000000000000000000000000000000000000001", 
                            2000
                          ]]"#;
        let (pairs, _) = parse_token_pairs(tokens);
        assert_eq!(
            pairs
                .get(
                    &(ordered_addresses(
                        Address::from_str(&"0x0000000000000000000000000000000000000001").unwrap(),
                        Address::from_str(&"0x0000000000000000000000000000000000000000").unwrap()
                    ))
                )
                .unwrap()
                .peek()
                .unwrap()
                .0,
            1000
        );
    }
}
