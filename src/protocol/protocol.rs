use super::config::Config;
use super::exactly_events::ExactlyEvents;
use super::fixed_point_math::{math, FixedPointMath, FixedPointMathGen};
use super::liquidation::{Liquidation, LiquidationData, Repay};
use crate::protocol::liquidation::LiquidationAction;
use crate::protocol::{Account, AggregatorProxy, Auditor, InterestRateModel, Market};
use ethers::abi::{Tokenizable, Tokenize};
use ethers::prelude::signer::SignerMiddlewareError;
use ethers::prelude::{
    abi::{Abi, RawLog},
    types::Filter,
    Address, EthLogDecode, LogMeta, Middleware, Multicall, Signer, SignerMiddleware, U256, U64,
};
use ethers::prelude::{Log, PubsubClient};
use eyre::Result;
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::str::FromStr;
use std::sync::Arc;
use std::{collections::HashMap, time::Duration};
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::Mutex;
use tokio::time;
use tokio_stream::StreamExt;

use super::{ExactlyOracle, LiquidationIncentive, Liquidator, MarketAccount, Previewer};

#[cfg(feature = "liquidation-stats")]
use crate::protocol::LiquidateFilter;

const DEFAULT_GAS_PRICE: U256 = math::make_u256(10_000u64);

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
    UpdateAll(Option<U64>),
    UpdateUser((Option<U64>, Address)),
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

// #[derive(Debug)]
pub struct Protocol<M, S> {
    client: Arc<SignerMiddleware<M, S>>,
    last_sync: (U64, i128, i128),
    previewer: Previewer<SignerMiddleware<M, S>>,
    auditor: Auditor<SignerMiddleware<M, S>>,
    oracle: ExactlyOracle<SignerMiddleware<M, S>>,
    markets: HashMap<Address, Market<M, S>>,
    sp_fee_rate: U256,
    accounts: HashMap<Address, Account>,
    contracts_to_listen: HashMap<ContractKey, Address>,
    comparison_enabled: bool,
    liquidation_incentive: LiquidationIncentive,
    market_weth_address: Address,
    liquidation: Arc<Mutex<Liquidation<M, S>>>,
    liquidation_sender: Sender<LiquidationData>,
    token_pairs: Arc<HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>>,
    tokens: Arc<HashSet<Address>>,
}

impl<M: 'static + Middleware, S: 'static + Signer> std::fmt::Debug for Protocol<M, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreditService").finish()
    }
}

impl<M: 'static + Middleware, S: 'static + Signer> Protocol<M, S> {
    async fn get_contracts(
        client: Arc<SignerMiddleware<M, S>>,
        client_relayer: Arc<SignerMiddleware<M, S>>,
        config: &Config,
    ) -> (
        u64,
        Auditor<SignerMiddleware<M, S>>,
        Previewer<SignerMiddleware<M, S>>,
        ExactlyOracle<SignerMiddleware<M, S>>,
        Liquidator<SignerMiddleware<M, S>>,
    ) {
        let (auditor_address, _, deployed_block) = Protocol::<M, S>::parse_abi(&format!(
            "node_modules/@exactly-protocol/protocol/deployments/{}/Auditor.json",
            config.chain_id_name
        ));

        let (previewer_address, _, _) = Protocol::<M, S>::parse_abi(&format!(
            "node_modules/@exactly-protocol/protocol/deployments/{}/Previewer.json",
            config.chain_id_name
        ));

        let (liquidator_address, _, _) = Protocol::<M, S>::parse_abi(&format!(
            "deployments/{}/Liquidator.json",
            config.chain_id_name
        ));

        let auditor = Auditor::new(auditor_address, Arc::clone(&client));
        let previewer = Previewer::new(previewer_address, Arc::clone(&client));
        let oracle = ExactlyOracle::new(Address::zero(), Arc::clone(&client));
        let liquidator = Liquidator::new(liquidator_address, Arc::clone(&client_relayer));
        (deployed_block, auditor, previewer, oracle, liquidator)
    }

    pub async fn new(
        client: Arc<SignerMiddleware<M, S>>,
        client_relayer: Arc<SignerMiddleware<M, S>>,
        config: &Config,
    ) -> Result<Protocol<M, S>> {
        let (deployed_block, auditor, previewer, oracle, liquidator) =
            Self::get_contracts(Arc::clone(&client), Arc::clone(&client_relayer), &config).await;

        let auditor_markets = auditor.all_markets().call().await?;
        let mut markets = HashMap::<Address, Market<M, S>>::new();
        for market in auditor_markets {
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

        let (market_weth_address, _, _) = Self::parse_abi(&format!(
            "node_modules/@exactly-protocol/protocol/deployments/{}/MarketWETH.json",
            config.chain_id_name
        ));

        let (liquidation_sender, liquidation_receiver) = mpsc::channel(1);

        let liquidation = Arc::new(Mutex::new(Liquidation::new(
            Arc::clone(&client),
            &config.token_pairs,
            liquidator,
            previewer.clone(),
            auditor.clone(),
            market_weth_address,
            config.backup,
        )));

        let liquidation_lock = liquidation.lock().await;
        let tokens = liquidation_lock.get_tokens();
        let token_pairs = liquidation_lock.get_token_pairs();
        drop(liquidation_lock);

        let liquidation_clone = Arc::clone(&liquidation);
        tokio::spawn(async move {
            let _ = Liquidation::run(liquidation_clone, liquidation_receiver).await;
        });

        Ok(Protocol {
            client: Arc::clone(&client),
            last_sync: (U64::from(deployed_block), -1, -1),
            auditor,
            previewer,
            oracle,
            markets,
            sp_fee_rate: U256::zero(),
            accounts: HashMap::new(),
            contracts_to_listen,
            comparison_enabled: config.comparison_enabled,
            liquidation_incentive: Default::default(),
            market_weth_address,
            liquidation,
            liquidation_sender,
            tokens,
            token_pairs,
        })
    }

    pub async fn update_client(
        &mut self,
        client: Arc<SignerMiddleware<M, S>>,
        client_relayer: Arc<SignerMiddleware<M, S>>,
        config: &Config,
    ) {
        let (_, auditor, previewer, oracle, liquidator) =
            Self::get_contracts(Arc::clone(&client), Arc::clone(&client_relayer), config).await;
        self.client = Arc::clone(&client);
        self.auditor = auditor;
        self.previewer = previewer;
        self.oracle = oracle;
        (*self.liquidation.lock().await).set_liquidator(liquidator);
        for market in self.markets.values_mut() {
            let address = market.contract.address();
            market.contract =
                crate::protocol::market_mod::Market::new(address, Arc::clone(&client));
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
            let mut last_gas_price = DEFAULT_GAS_PRICE;
            let mut user = None;
            loop {
                let d = Duration::from_millis(2_000);
                match time::timeout(d, debounce_rx.recv()).await {
                    Ok(Some(activity)) => match activity {
                        TaskActivity::StartCheckLiquidation => check_liquidations = true,
                        TaskActivity::StopCheckLiquidation => check_liquidations = false,
                        TaskActivity::UpdateAll(block) => {
                            block_number = block;
                            liquidation_checked = false;
                            user = None;
                        }
                        TaskActivity::UpdateUser((block, user_to_check)) => {
                            block_number = block;
                            liquidation_checked = false;
                            user = Some(user_to_check);
                        }
                    },
                    Ok(None) => {}
                    Err(_) => {
                        if check_liquidations && !liquidation_checked {
                            if let Some(block) = block_number {
                                let lock = me
                                    .lock()
                                    .await
                                    .check_liquidations(block, &mut last_gas_price, user)
                                    .await;
                                liquidation_checked = true;
                                drop(lock);
                            }
                        }
                    }
                }
            }
        });
        let file = File::create("data.log").unwrap();
        let mut writer = BufWriter::new(file);
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
                if getting_logs {
                    filter = filter.to_block(last_block);
                    let result = client.get_logs(&filter).await;
                    if let Ok(logs) = result {
                        (None, Some(logs))
                    } else {
                        a.abort();
                        break 'filter;
                    }
                } else {
                    (Some(client.subscribe_logs(&filter).await), None)
                }
            };
            match result {
                (None, Some(logs)) => {
                    _ = debounce_tx.send(TaskActivity::StopCheckLiquidation).await;
                    let mut me = service.lock().await;
                    for log in logs {
                        let status = me.handle_log(log, &debounce_tx, &mut writer).await;
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
                    if last_block >= latest_block {
                        first_block = last_block + 1u64;
                        getting_logs = false;
                    } else {
                        first_block = last_block + 1u64;
                        last_block = if first_block + PAGE_SIZE > latest_block {
                            latest_block
                        } else {
                            last_block + PAGE_SIZE
                        };
                    }
                }
                (Some(stream), None) => {
                    _ = debounce_tx.send(TaskActivity::StartCheckLiquidation).await;
                    match stream {
                        Ok(mut stream) => {
                            _ = std::io::Write::flush(&mut writer);
                            while let Some(log) = stream.next().await {
                                let mut me = service.lock().await;
                                let status = me.handle_log(log, &debounce_tx, &mut writer).await;
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
        let result = multicall.block(block).call_raw().await?;
        for (i, market) in self.markets.values_mut().filter(|m| m.listed).enumerate() {
            market.oracle_price = result[i].clone().into_uint().unwrap();
        }

        Ok(())
    }

    // handle a new received log
    async fn handle_log(
        &mut self,
        log: Log,
        sender: &Sender<TaskActivity>,
        _writer: &mut BufWriter<File>,
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
        println!("{:?} | {:?}", event, meta);
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
                    .send(TaskActivity::UpdateAll(log.block_number))
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
                let result = multicall.block(meta.block_number).call().await;
                if let Ok((floating_a, floating_b, floating_max_utilization)) = result {
                    market.floating_a = floating_a;
                    market.floating_b = floating_b;
                    market.floating_max_utilization = floating_max_utilization;
                }
                self.contracts_to_listen.insert(
                    ContractKey {
                        address: data.market,
                        kind: ContractKeyKind::Market,
                    },
                    data.market,
                );
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
                    market.floating_a = floating_a;
                    market.floating_b = floating_b;
                    market.floating_max_utilization = floating_max_utilization;
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
                #[cfg(feature = "liquidation-stats")]
                self.handle_liquidate_event(&meta, &data, _writer).await?;

                sender
                    .send(TaskActivity::UpdateUser((log.block_number, data.borrower)))
                    .await?;
                return Ok(LogIterating::NextLog);
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
                    .send(TaskActivity::UpdateAll(log.block_number))
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
        sender
            .send(TaskActivity::UpdateAll(log.block_number))
            .await?;
        Ok(LogIterating::NextLog)
    }

    async fn check_liquidations(
        &self,
        block_number: U64,
        last_gas_price: &mut U256,
        user: Option<Address>,
    ) -> Result<()> {
        println!("check_liquidations");
        let block = self
            .client
            .provider()
            .get_block(block_number)
            .await?
            .unwrap();
        *last_gas_price =
            block.base_fee_per_gas.unwrap_or(*last_gas_price) + U256::from(1_500_000_000u128);

        let to_timestamp = block.timestamp;

        if self.comparison_enabled {
            println!("comparison_enabled");
            self.compare_accounts(block_number, to_timestamp).await?;
        }

        let mut liquidations: HashMap<Address, (Account, Repay)> = HashMap::new();
        if let Some(address) = &user {
            let account = &self.accounts[address];
            self.check_liquidations_on_account(
                address,
                account,
                to_timestamp,
                last_gas_price,
                &mut liquidations,
            );
        } else {
            for (address, account) in &self.accounts {
                self.check_liquidations_on_account(
                    address,
                    account,
                    to_timestamp,
                    last_gas_price,
                    &mut liquidations,
                );
            }
        }
        println!(
            "accounts to check for liquidations {:#?}",
            self.accounts.len()
        );
        println!("accounts to liquidate {:#?}", liquidations.len());
        if liquidations.len() > 0 {
            self.liquidation_sender
                .send(LiquidationData {
                    liquidations,
                    eth_price: self.markets[&self.market_weth_address].oracle_price,
                    gas_price: *last_gas_price,
                    liquidation_incentive: self.liquidation_incentive.clone(),
                    action: if user == None {
                        LiquidationAction::Update
                    } else {
                        LiquidationAction::Insert
                    },
                    markets: self.markets.keys().cloned().collect(),
                    assets: self
                        .markets
                        .iter()
                        .map(|(address, market)| (*address, market.asset))
                        .collect(),
                    oracle: self.oracle.address(),
                })
                .await?;
        }
        Ok(())
    }

    fn check_liquidations_on_account(
        &self,
        address: &Address,
        account: &Account,
        to_timestamp: U256,
        last_gas_price: &U256,
        liquidations: &mut HashMap<Address, (Account, Repay)>,
    ) {
        let hf = Self::compute_hf(&self.markets, account, to_timestamp);
        if let Ok((hf, _, _, repay)) = hf {
            if hf < math::WAD && repay.total_adjusted_debt != U256::zero() {
                if let Some((profitable, _, _, _)) = Liquidation::<M, S>::is_profitable(
                    &repay,
                    &self.liquidation_incentive,
                    *last_gas_price,
                    self.markets[&self.market_weth_address].oracle_price,
                    &self.token_pairs,
                    &self.tokens,
                ) {
                    if profitable {
                        liquidations.insert(address.clone(), (account.clone(), repay));
                    }
                }
            }
        }
    }

    #[cfg(feature = "liquidation-stats")]
    async fn handle_liquidate_event(
        &self,
        meta: &LogMeta,
        data: &LiquidateFilter,
        writer: &mut BufWriter<File>,
    ) -> Result<()> {
        use ethers::types::{Block, TransactionReceipt, H256};
        use futures::TryFutureExt;
        use std::{collections::BTreeMap, io::Write};

        let mut previous_multicall =
            Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None).await?;
        previous_multicall.add_call(self.auditor.account_liquidity(
            data.borrower,
            Address::zero(),
            U256::zero(),
        ));
        previous_multicall.add_call(self.auditor.liquidation_incentive());
        previous_multicall.add_call(self.previewer.exactly(data.borrower));
        let previous_multicall = previous_multicall.block(meta.block_number - 1u64);

        let mut current_multicall =
            Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None).await?;
        current_multicall.add_call(self.auditor.account_liquidity(
            data.borrower,
            Address::zero(),
            U256::zero(),
        ));
        current_multicall.add_call(self.auditor.liquidation_incentive());
        let current_multicall = current_multicall.block(meta.block_number);

        let mut price_multicall =
            Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None).await?;
        for market in self.markets.keys() {
            price_multicall.add_call(self.oracle.asset_price(*market));
        }
        let price_multicall = price_multicall.block(meta.block_number - 1u64);

        let response = tokio::try_join!(
            previous_multicall.call(),
            current_multicall.call(),
            price_multicall.call_raw(),
            self.client
                .provider()
                .get_block(meta.block_number)
                .map_err(|_| ethers::prelude::ContractError::ConstructorError),
            self.client
                .provider()
                .get_transaction_receipt(meta.transaction_hash)
                .map_err(|_| ethers::prelude::ContractError::ConstructorError),
        );
        let (previous, current, prices, current_block_data, receipt): (
            _,
            _,
            _,
            Option<Block<H256>>,
            Option<TransactionReceipt>,
        ) = if let Ok(response) = response {
            response
        } else {
            println!("error getting data from contracts on current block");
            return Ok(());
        };
        let current_block_data = current_block_data.unwrap();
        let timestamp = current_block_data.timestamp;
        let receipt = receipt.unwrap();
        let gas_cost = receipt.gas_used.unwrap();
        let gas_price = gas_cost
            * receipt
                .effective_gas_price
                .unwrap()
                .mul_wad_down(self.markets[&self.market_weth_address].oracle_price);

        let market_prices: HashMap<Address, U256> = self
            .markets
            .keys()
            .zip(prices)
            .map(|(market, price)| (*market, price.into_uint().unwrap()))
            .collect();

        let ((current_adjusted_collateral, current_adjusted_debt), (_, _)): (
            (U256, U256),
            (u128, u128),
        ) = current;
        let (
            (previous_adjusted_collateral, previous_adjusted_debt),
            (previous_liquidator, previous_lenders),
            previous_market_account,
        ): ((U256, U256), (u128, u128), Vec<MarketAccount>) = previous;

        let mut total_debt = U256::zero();
        let mut total_collateral = U256::zero();
        let mut total_supply_per_market = BTreeMap::new();
        for market_account in previous_market_account {
            let total_float_supply = market_account.total_floating_deposit_assets;
            if market_account.is_collateral {
                total_collateral += market_account.floating_deposit_assets.mul_div_down(
                    market_prices[&market_account.market],
                    U256::exp10(market_account.decimals as usize),
                )
            };

            let mut total_fixed_supply = U256::zero();
            for fixed_pool in market_account.fixed_pools {
                total_fixed_supply += fixed_pool.supplied;
            }
            let mut market_debt = U256::zero();
            for fixed_position in market_account.fixed_borrow_positions {
                let borrowed = fixed_position.position.principal + fixed_position.position.fee;
                market_debt += borrowed;
                if U256::from(fixed_position.maturity) < timestamp {
                    market_debt += borrowed.mul_wad_down(
                        (timestamp - U256::from(fixed_position.maturity))
                            * U256::from(market_account.penalty_rate),
                    )
                }
            }
            market_debt += market_account.floating_borrow_assets;
            total_debt += market_debt.mul_div_up(
                market_prices[&market_account.market],
                U256::exp10(market_account.decimals as usize),
            );
            total_supply_per_market.insert(
                market_account.market,
                (
                    market_account.asset_symbol,
                    (
                        total_float_supply.mul_div_down(
                            market_prices[&market_account.market],
                            U256::exp10(market_account.decimals as usize),
                        ),
                        (total_float_supply + total_fixed_supply).mul_div_down(
                            market_prices[&market_account.market],
                            U256::exp10(market_account.decimals as usize),
                        ),
                    ),
                ),
            );
        }

        let previous_liquidation_incentive = LiquidationIncentive {
            liquidator: previous_liquidator,
            lenders: previous_lenders,
        };
        let (_, _, _, repay) =
            Self::compute_hf(&self.markets, &self.accounts[&data.borrower], timestamp)?;

        let close_factor =
            Liquidation::<M, S>::calculate_close_factor(&repay, &previous_liquidation_incentive);
        let current_hf = if current_adjusted_debt > U256::zero() {
            current_adjusted_collateral.div_wad_down(current_adjusted_debt)
        } else {
            U256::zero()
        };
        let previous_hf = if previous_adjusted_debt > U256::zero() {
            previous_adjusted_collateral.div_wad_down(previous_adjusted_debt)
        } else {
            U256::zero()
        };
        let liquidators_fee = data.seized_assets * U256::from(previous_liquidator);
        writer.write_fmt(format_args!(
            "{:#?},{:#?},{:#?},{:#?},{:#?},{:#?},{:#?},{:#?},{:#?},{:#?},{:#?},{:#?},{:#?}",
            timestamp,
            data.borrower,
            total_collateral,
            total_debt,
            previous_hf,
            close_factor,
            data.seized_assets,
            data.assets,
            current_hf,
            liquidators_fee,
            data.lenders_assets,
            gas_cost,
            gas_price
        ))?;
        for (_, (symbol, supply)) in total_supply_per_market {
            writer.write_fmt(format_args!(
                ",{:#?},{:#?},{:#?}",
                symbol, supply.0, supply.1
            ))?;
        }
        writer.write_fmt(format_args!("\n"))?;
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
                        account.fixed_deposit_positions[&fixed_deposit.maturity]
                    );
                }
                for fixed_borrow in &market_account.fixed_borrow_positions {
                    success &= compare!(
                        format!("fixed_borrow_positions[{}]", fixed_borrow.maturity),
                        address,
                        market_account.market,
                        fixed_borrow.position.principal + fixed_borrow.position.fee,
                        account.fixed_borrow_positions[&fixed_borrow.maturity]
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
                if value >= repay.market_to_seize_value {
                    repay.market_to_seize_value = value;
                    repay.market_to_seize = Some(*market_address);
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
            if value >= repay.market_to_liquidate_debt {
                repay.price = market.oracle_price;
                repay.decimals = market.decimals;
                repay.market_to_liquidate_debt = value;
                repay.market_to_repay = Some(*market_address);
            }
        }
        repay.total_value_collateral = total_collateral;
        repay.total_adjusted_collateral = adjusted_collateral;
        repay.total_value_debt = total_debt;
        repay.total_adjusted_debt = adjusted_debt;
        repay.repay_asset_address = markets[&repay.market_to_repay.unwrap()].asset;
        if let Some(seizable_collateral) = &repay.market_to_seize {
            repay.collateral_asset_address = markets[seizable_collateral].asset;
        }
        let hf = if adjusted_debt == U256::zero() {
            adjusted_collateral
        } else {
            math::WAD * adjusted_collateral / adjusted_debt
        };
        if hf < math::WAD && adjusted_debt != U256::zero() {
            println!("==============");
            println!("Account                {:?}", account.address);
            println!("Total Collateral   USD {:?}", total_collateral);
            println!("Total Debt         USD {:?}", total_debt);
            println!("Health factor {:?}\n", hf);
        }
        Ok((hf, adjusted_collateral, adjusted_debt, repay))
    }
}
