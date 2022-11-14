use super::config::Config;
use super::exactly_events::ExactlyEvents;
use super::fixed_point_math::{math, FixedPointMath, FixedPointMathGen};
use super::liquidation::{Liquidation, LiquidationData, Repay};
use crate::generate_abi::{
    AggregatorProxy, Auditor, InterestRateModel, LiquidationIncentive, Previewer, PriceFeed,
    PriceFeedLido, PriceFeedWrapper,
};
use crate::liquidation::{LiquidationAction, ProtocolState};
use crate::market::{PriceFeedController, PriceRate};
use crate::{Account, Market};
use ethers::abi::Tokenize;
use ethers::prelude::builders::ContractCall;
use ethers::prelude::signer::SignerMiddlewareError;
use ethers::prelude::{
    abi::{Abi, RawLog},
    types::Filter,
    Address, EthLogDecode, LogMeta, Middleware, Multicall, Signer, SignerMiddleware, U256, U64,
};
use ethers::prelude::{Log, MulticallVersion, PubsubClient};
use ethers::providers::SubscriptionStream;
use eyre::Result;
use serde_json::Value;
use std::backtrace::Backtrace;
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

#[cfg(feature = "complete-compare")]
use crate::generate_abi::MarketAccount;

#[cfg(feature = "liquidation-stats")]
use crate::protocol::LiquidateFilter;

const DEFAULT_GAS_PRICE: U256 = math::make_u256(10_000u64);
pub const BASE_FEED: &str = "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE";

pub type TokenPairFeeMap = HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>;

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
enum ContractKeyKind {
    Market,
    PriceFeed,
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
            println!("{:?}@{}.{}", $account, $market, $label);
            println!("reference: {:#?}", $ref);
            println!("    value: {:#?}", $val);
            false
        } else {
            true
        }
    };
}

// #[derive(Debug)]
pub struct Protocol<M, W, S> {
    client: Arc<SignerMiddleware<M, S>>,
    last_sync: (U64, i128, i128),
    previewer: Previewer<SignerMiddleware<M, S>>,
    auditor: Auditor<SignerMiddleware<M, S>>,
    markets: HashMap<Address, Market<M, S>>,
    sp_fee_rate: U256,
    accounts: HashMap<Address, Account>,
    contracts_to_listen: HashMap<ContractKey, Address>,
    comparison_enabled: bool,
    liquidation_incentive: LiquidationIncentive,
    market_weth_address: Address,
    liquidation: Arc<Mutex<Liquidation<M, W, S>>>,
    liquidation_sender: Sender<LiquidationData>,
    token_pairs: Arc<TokenPairFeeMap>,
    tokens: Arc<HashSet<Address>>,
    price_decimals: U256,
    repay_offset: U256,
}

impl<M: 'static + Middleware, W: 'static + Middleware, S: 'static + Signer> std::fmt::Debug
    for Protocol<M, W, S>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreditService").finish()
    }
}

impl<M: 'static + Middleware, W: 'static + Middleware, S: 'static + Signer> Protocol<M, W, S> {
    async fn get_contracts(
        client: Arc<SignerMiddleware<M, S>>,
        config: &Config,
    ) -> (
        u64,
        Auditor<SignerMiddleware<M, S>>,
        Previewer<SignerMiddleware<M, S>>,
    ) {
        let (auditor_address, _, deployed_block) = Protocol::<M, W, S>::parse_abi(&format!(
            "node_modules/@exactly-protocol/protocol/deployments/{}/Auditor.json",
            config.chain_id_name
        ));

        let (previewer_address, _, _) = Protocol::<M, W, S>::parse_abi(&format!(
            "node_modules/@exactly-protocol/protocol/deployments/{}/Previewer.json",
            config.chain_id_name
        ));

        let auditor = Auditor::new(auditor_address, Arc::clone(&client));
        let previewer = Previewer::new(previewer_address, Arc::clone(&client));
        (deployed_block, auditor, previewer)
    }

    pub async fn new(
        client: Arc<SignerMiddleware<M, S>>,
        client_relayer: Arc<SignerMiddleware<W, S>>,
        config: &Config,
    ) -> Result<Protocol<M, W, S>> {
        let (deployed_block, auditor, previewer) =
            Self::get_contracts(Arc::clone(&client), config).await;

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
            Arc::clone(&client_relayer),
            &config.token_pairs,
            previewer.clone(),
            auditor.clone(),
            market_weth_address,
            config,
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
            price_decimals: U256::zero(),
            repay_offset: config.repay_offset,
        })
    }

    pub async fn update_client(
        &mut self,
        client: Arc<SignerMiddleware<M, S>>,
        client_relayer: Arc<SignerMiddleware<W, S>>,
        config: &Config,
    ) {
        let (_, auditor, previewer) = Self::get_contracts(Arc::clone(&client), config).await;
        self.client = Arc::clone(&client);
        self.auditor = auditor;
        self.previewer = previewer;
        (*self.liquidation.lock().await)
            .set_liquidator(Arc::clone(&client_relayer), config.chain_id_name.clone());
        for market in self.markets.values_mut() {
            let address = market.contract.address();
            market.contract = crate::generate_abi::market_protocol::MarketProtocol::new(
                address,
                Arc::clone(&client),
            );
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
        enum DataFrom<'a, M: Middleware, S: Signer>
        where
            <M as Middleware>::Provider: PubsubClient,
        {
            Log(Vec<Log>),
            Stream(Result<SubscriptionStream<'a, M::Provider, Log>, SignerMiddlewareError<M, S>>),
        }
        let file = File::create("data.log").unwrap();
        let mut writer = BufWriter::new(file);
        let mut first_block = service.lock().await.last_sync.0;
        let mut last_block = first_block + PAGE_SIZE;
        let mut latest_block = U64::zero();
        let mut getting_logs = true;
        'filter: loop {
            let client;
            let result: DataFrom<M, S> = {
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
                        DataFrom::Log(logs)
                    } else {
                        a.abort();
                        break 'filter;
                    }
                } else {
                    DataFrom::Stream(client.subscribe_logs(&filter).await)
                }
            };
            match result {
                DataFrom::Log(logs) => {
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
                        first_block = latest_block;
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
                DataFrom::Stream(stream) => {
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
                        Err(e) => {
                            println!("Error from stream: {:#?}", Backtrace::force_capture());
                            if let SignerMiddlewareError::MiddlewareError(m) = &e {
                                println!("error to subscribe (middleware): {:#?}", m);
                            }
                            panic!("subscribe disconnection: {:#?}", e);
                        }
                    }
                }
            };
        }
        a.abort();
        // if error {
        //     return Err(Arc::try_unwrap(service).unwrap().into_inner());
        // }
        Ok(Arc::try_unwrap(service).unwrap().into_inner())
    }

    async fn update_price_lido(
        price_feed: &mut PriceFeedController,
        main_price: U256,
        block: U64,
        client: Arc<SignerMiddleware<M, S>>,
        price_decimals: U256,
    ) -> Result<U256> {
        let wrapper = &mut price_feed.wrapper.unwrap();
        let price_feed_contract = PriceFeedLido::new(wrapper.address, Arc::clone(&client));
        let conversion_selector_method: ContractCall<SignerMiddleware<M, S>, U256> =
            price_feed_contract.method_hash(wrapper.conversion_selector, wrapper.base_unit)?;
        let rate: U256 = conversion_selector_method.block(block).call().await?;
        wrapper.rate = rate;
        wrapper.main_price = main_price;
        Ok(rate.mul_div_down(main_price, wrapper.base_unit)
            * U256::exp10(18 - price_decimals.as_usize()))
    }

    async fn update_prices(&mut self, block: U64) -> Result<()> {
        let price_feed: Vec<PriceFeed<SignerMiddleware<M, S>>> = self
            .markets
            .iter()
            .filter(|(_, v)| v.listed && v.price_feed.is_some() && !v.base_market)
            .map(|(_, market)| {
                PriceFeed::new(
                    market.price_feed.as_ref().unwrap().address,
                    self.client.clone(),
                )
            })
            .collect();
        let mut multicall =
            Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None).await?;
        if price_feed.is_empty() {
            return Ok(());
        }
        price_feed.iter().for_each(|price_feed_wrapper| {
            multicall.add_call(price_feed_wrapper.latest_answer(), false);
        });
        let results = multicall
            .version(MulticallVersion::Multicall)
            .block(block)
            .call_raw()
            .await?;

        for (market, result) in self
            .markets
            .values_mut()
            .filter(|m| m.listed && m.price_feed.is_some() && !m.base_market)
            .zip(results)
        {
            let value = result.clone().into_int().unwrap();
            let price = if market.price_feed.as_ref().and_then(|p| p.wrapper).is_some() {
                Self::update_price_lido(
                    market.price_feed.as_mut().unwrap(),
                    value,
                    block,
                    Arc::clone(&self.client),
                    self.price_decimals,
                )
                .await?
            } else {
                value * U256::exp10(18 - self.price_decimals.as_usize())
            };
            market.price = price;
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
        if result.is_err() {
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
            ExactlyEvents::MaxFuturePoolsSet(data) => {
                self.markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| Market::new(*key, &self.client))
                    .max_future_pools = data.max_future_pools.as_u32() as u8;
            }

            ExactlyEvents::EarningsAccumulatorSmoothFactorSet(data) => {
                self.markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| Market::new(*key, &self.client))
                    .earnings_accumulator_smooth_factor = data.earnings_accumulator_smooth_factor;
            }

            ExactlyEvents::MarketListed(data) => {
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
                multicall.add_call(market.contract.max_future_pools(), false);
                multicall.add_call(market.contract.earnings_accumulator_smooth_factor(), false);
                multicall.add_call(market.contract.interest_rate_model(), false);
                multicall.add_call(market.contract.penalty_rate(), false);
                multicall.add_call(market.contract.treasury_fee_rate(), false);
                multicall.add_call(market.contract.asset(), false);
                multicall.add_call(market.contract.last_accumulator_accrual(), false);
                multicall.add_call(market.contract.last_floating_debt_update(), false);
                multicall = multicall
                    .version(MulticallVersion::Multicall)
                    .block(meta.block_number);
                let (
                    max_future_pools,
                    earnings_accumulator_smooth_factor,
                    interest_rate_model,
                    penalty_rate,
                    treasury_fee_rate,
                    asset,
                    last_accumulator_accrual,
                    last_floating_debt_update,
                ) = multicall.call().await?;
                market.max_future_pools = max_future_pools;
                market.earnings_accumulator_smooth_factor = earnings_accumulator_smooth_factor;
                market.interest_rate_model = interest_rate_model;
                market.penalty_rate = penalty_rate;
                market.treasury_fee_rate = treasury_fee_rate;
                market.asset = asset;
                market.last_accumulator_accrual = last_accumulator_accrual;
                market.last_floating_debt_update = last_floating_debt_update;

                let irm =
                    InterestRateModel::new(market.interest_rate_model, Arc::clone(&self.client));
                multicall.clear_calls();
                multicall.add_call(irm.floating_curve_a(), false);
                multicall.add_call(irm.floating_curve_b(), false);
                multicall.add_call(irm.floating_max_utilization(), false);
                let result = multicall
                    .version(MulticallVersion::Multicall)
                    .block(meta.block_number)
                    .call()
                    .await;
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

            ExactlyEvents::InterestRateModelSet(data) => {
                let mut market = self.markets.get_mut(&meta.address).unwrap();
                market.interest_rate_model = data.interest_rate_model;
                let irm =
                    InterestRateModel::new(market.interest_rate_model, Arc::clone(&self.client));
                let mut multicall =
                    Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None)
                        .await
                        .unwrap();

                multicall.add_call(irm.floating_curve_a(), false);
                multicall.add_call(irm.floating_curve_b(), false);
                multicall.add_call(irm.floating_max_utilization(), false);
                let result = multicall
                    .version(MulticallVersion::Multicall)
                    .block(meta.block_number)
                    .call()
                    .await;
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

            ExactlyEvents::Transfer(data) => {
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
            ExactlyEvents::Borrow(data) => {
                self.accounts
                    .entry(data.borrower)
                    .or_insert_with_key(|key| Account::new(*key, &self.markets))
                    .positions
                    .entry(meta.address)
                    .or_default()
                    .floating_borrow_shares += data.shares;
            }
            ExactlyEvents::Repay(data) => {
                self.accounts
                    .entry(data.borrower)
                    .or_insert_with_key(|key| Account::new(*key, &self.markets))
                    .positions
                    .entry(meta.address)
                    .or_default()
                    .floating_borrow_shares -= data.shares;
            }
            ExactlyEvents::DepositAtMaturity(data) => {
                self.accounts
                    .entry(data.owner)
                    .or_insert_with(|| Account::new(data.owner, &self.markets))
                    .deposit_at_maturity(&data, &meta.address);
            }
            ExactlyEvents::WithdrawAtMaturity(data) => {
                self.accounts
                    .entry(data.owner)
                    .or_insert_with(|| Account::new(data.owner, &self.markets))
                    .withdraw_at_maturity(data, &meta.address);
            }
            ExactlyEvents::BorrowAtMaturity(data) => {
                self.accounts
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.markets))
                    .borrow_at_maturity(&data, &meta.address);
            }
            ExactlyEvents::RepayAtMaturity(data) => {
                self.accounts
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.markets))
                    .repay_at_maturity(&data, &meta.address);
            }
            ExactlyEvents::Liquidate(data) => {
                #[cfg(feature = "liquidation-stats")]
                self.handle_liquidate_event(&meta, &data, _writer).await?;

                sender
                    .send(TaskActivity::UpdateUser((log.block_number, data.borrower)))
                    .await?;
                return Ok(LogIterating::NextLog);
            }
            ExactlyEvents::Seize(data) => {
                self.accounts
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.markets))
                    .asset_seized(data, &meta.address);
            }

            ExactlyEvents::AdjustFactorSet(data) => {
                self.markets
                    .entry(data.market)
                    .or_insert_with_key(|key| Market::new(*key, &self.client))
                    .adjust_factor = data.adjust_factor;
            }

            ExactlyEvents::PenaltyRateSet(data) => {
                self.markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| Market::new(*key, &self.client))
                    .penalty_rate = data.penalty_rate;
            }

            ExactlyEvents::MarketEntered(data) => {
                self.accounts
                    .entry(data.account)
                    .or_insert_with(|| Account::new(data.account, &self.markets))
                    .set_collateral(&data.market);
            }

            ExactlyEvents::MarketExited(data) => {
                self.accounts
                    .entry(data.account)
                    .or_insert_with(|| Account::new(data.account, &self.markets))
                    .unset_collateral(&data.market);
            }

            ExactlyEvents::BackupFeeRateSet(data) => {
                self.sp_fee_rate = data.backup_fee_rate;
                for market in self.markets.values_mut() {
                    market.smart_pool_fee_rate = data.backup_fee_rate;
                }
            }

            ExactlyEvents::PriceFeedSetFilter(data) => {
                if data.price_feed == Address::from_str(BASE_FEED).unwrap() {
                    let market = self
                        .markets
                        .entry(data.market)
                        .or_insert_with_key(|key| Market::new(*key, &self.client));

                    market.price_feed =
                        Some(PriceFeedController::main_price_feed(data.price_feed, None));
                    market.price = U256::exp10(self.price_decimals.as_usize());
                    market.base_market = true;
                    return Ok(LogIterating::NextLog);
                }
                self.handle_price_feed(data, &meta).await?;
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

            ExactlyEvents::AnswerUpdated(data) => {
                self.markets.iter_mut().for_each(|(_, market)| {
                    if let Some(price_feed) = market.price_feed.as_mut().filter(|p| {
                        if let Some(event_emitter) = p.event_emitter {
                            event_emitter == meta.address
                        } else {
                            false
                        }
                    }) {
                        let price = if let Some(wrapper) = price_feed.wrapper.as_mut() {
                            wrapper.main_price = data.current.into_raw();
                            // * U256::exp10(18 - self.price_decimals.as_usize());
                            wrapper
                                .rate
                                .mul_div_down(wrapper.main_price, wrapper.base_unit)
                                * U256::exp10(18 - self.price_decimals.as_usize())
                        } else {
                            data.current.into_raw()
                                * U256::exp10(18 - self.price_decimals.as_usize())
                        };
                        market.price = price;
                    };
                });
            }

            ExactlyEvents::MarketUpdate(data) => {
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

            ExactlyEvents::FloatingDebtUpdate(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address, &self.client));
                market.floating_utilization = data.utilization;
                market.last_floating_debt_update = data.timestamp;
            }

            ExactlyEvents::AccumulatorAccrual(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address, &self.client));
                market.last_accumulator_accrual = data.timestamp;
            }

            ExactlyEvents::FixedEarningsUpdate(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address, &self.client));

                let pool = market.fixed_pools.entry(data.maturity).or_default();
                pool.last_accrual = data.timestamp;
                pool.unassigned_earnings = data.unassigned_earnings;
            }

            ExactlyEvents::TreasurySet(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address, &self.client));
                market.treasury_fee_rate = data.treasury_fee_rate;
            }

            ExactlyEvents::LiquidationIncentiveSet(data) => {
                self.liquidation_incentive = data.liquidation_incentive;
            }

            ExactlyEvents::Initialized(_) => {
                self.price_decimals = self
                    .auditor
                    .price_decimals()
                    .block(meta.block_number)
                    .call()
                    .await?;
            }

            ExactlyEvents::PostTotalShares(data) => {
                self.markets
                    .iter_mut()
                    .filter(|(_, market)| {
                        if let Some(price_feed) = market.price_feed.as_ref() {
                            price_feed.wrapper.is_some()
                        } else {
                            false
                        }
                    })
                    .for_each(|(_, market)| {
                        let wrapper = market
                            .price_feed
                            .as_mut()
                            .unwrap()
                            .wrapper
                            .as_mut()
                            .unwrap();
                        wrapper.rate = if data.total_shares == 0.into() {
                            U256::zero()
                        } else {
                            wrapper.base_unit * data.post_total_pooled_ether / data.total_shares
                        };
                        market.price = wrapper
                            .rate
                            .mul_div_down(wrapper.main_price, wrapper.base_unit)
                            * U256::exp10(18 - self.price_decimals.as_usize());
                    });
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

    async fn handle_price_feed(
        &mut self,
        data: crate::generate_abi::PriceFeedSetFilter,
        meta: &LogMeta,
    ) -> Result<(), eyre::ErrReport> {
        let mut price_feed = PriceFeedController::default();
        price_feed.address = data.price_feed;

        let wrapper = PriceFeedWrapper::new(data.price_feed, Arc::clone(&self.client));
        let mut wrapper_multicall =
            Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None).await?;
        wrapper_multicall.add_call(wrapper.main_price_feed(), false);
        wrapper_multicall.add_call(wrapper.wrapper(), false);
        wrapper_multicall.add_call(wrapper.conversion_selector(), false);
        wrapper_multicall.add_call(wrapper.base_unit(), false);
        let price_feed_address = if let Ok(wrapper_price_feed) = wrapper_multicall
            .version(MulticallVersion::Multicall)
            .block(meta.block_number)
            .call()
            .await
        {
            let (main_price_feed, wrapper, conversion_selector, base_unit): (
                Address,
                Address,
                [u8; 4],
                U256,
            ) = wrapper_price_feed;
            price_feed.main_price_feed = Some(Box::new(PriceFeedController::main_price_feed(
                main_price_feed,
                None,
            )));
            price_feed.wrapper = Some(PriceRate {
                address: wrapper,
                conversion_selector,
                base_unit,
                main_price: U256::zero(),
                rate: U256::zero(),
                event_emitter: None,
            });
            main_price_feed
        } else {
            data.price_feed
        };
        let event_emitter = if let Ok(event_emitter) =
            AggregatorProxy::new(price_feed_address, Arc::clone(&self.client))
                .aggregator()
                .block(meta.block_number)
                .call()
                .await
        {
            event_emitter
        } else {
            price_feed_address
        };
        price_feed.event_emitter = Some(event_emitter);
        self.contracts_to_listen.insert(
            ContractKey {
                address: data.market,
                kind: ContractKeyKind::PriceFeed,
            },
            event_emitter,
        );
        let market = self
            .markets
            .entry(data.market)
            .or_insert_with_key(|key| Market::new(*key, &self.client));
        market.price_feed = Some(price_feed);
        if let Some(wrapper) = market
            .price_feed
            .as_mut()
            .and_then(|price_feed| price_feed.wrapper)
            .as_mut()
        {
            let lido = PriceFeedLido::new(wrapper.address, Arc::clone(&self.client));
            let oracle = lido.get_oracle().block(meta.block_number).call().await?;
            wrapper.event_emitter = Some(oracle);
            self.contracts_to_listen.insert(
                ContractKey {
                    address: wrapper.address,
                    kind: ContractKeyKind::PriceFeed,
                },
                oracle,
            );
        };

        Ok(())
    }

    async fn check_liquidations(
        &self,
        block_number: U64,
        last_gas_price: &mut U256,
        user: Option<Address>,
    ) -> Result<()> {
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
            "accounts checked for liquidations {}. {} {} found",
            self.accounts.len(),
            liquidations.len(),
            if liquidations.len() > 1 {
                "liquidations"
            } else {
                "liquidation"
            },
        );
        if !liquidations.is_empty() {
            self.liquidation_sender
                .send(LiquidationData {
                    liquidations,
                    action: if user.is_none() {
                        LiquidationAction::Update
                    } else {
                        LiquidationAction::Insert
                    },
                    state: ProtocolState {
                        gas_price: *last_gas_price,
                        markets: self.markets.keys().cloned().collect(),
                        assets: self
                            .markets
                            .iter()
                            .map(|(address, market)| (*address, market.asset))
                            .collect(),
                        price_feeds: self
                            .markets
                            .iter()
                            .map(|(address, market)| {
                                (*address, market.price_feed.as_ref().unwrap().address)
                            })
                            .collect(),
                        price_decimals: self.price_decimals,
                    },
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
                if let Some((profitable, _, _, _, _)) = Liquidation::<M, W, S>::is_profitable(
                    &repay,
                    &self.liquidation_incentive,
                    *last_gas_price,
                    self.markets[&self.market_weth_address].price,
                    &self.token_pairs,
                    &self.tokens,
                    self.repay_offset,
                ) {
                    if profitable {
                        liquidations.insert(*address, (account.clone(), repay));
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
        let previous_multicall = previous_multicall
            .version(MulticallVersion::Multicall)
            .block(meta.block_number - 1u64);

        let mut current_multicall =
            Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None).await?;
        current_multicall.add_call(self.auditor.account_liquidity(
            data.borrower,
            Address::zero(),
            U256::zero(),
        ));
        current_multicall.add_call(self.auditor.liquidation_incentive());
        let current_multicall = current_multicall
            .version(MulticallVersion::Multicall)
            .block(meta.block_number);

        let mut price_multicall =
            Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None).await?;
        for market in self.markets.keys() {
            price_multicall.add_call(self.oracle.asset_price(*market));
        }
        let price_multicall = price_multicall
            .version(MulticallVersion::Multicall)
            .block(meta.block_number - 1u64);

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

    #[cfg(feature = "complete-compare")]
    async fn multicall_previewer(
        &self,
        block: U64,
    ) -> HashMap<Address, HashMap<Address, MarketAccount>> {
        use ethers::abi::Tokenizable;

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

                multicall.add_call(self.previewer.exactly(*account), false);
            }
            let responses = multicall
                .version(MulticallVersion::Multicall)
                .block(block)
                .call_raw()
                .await;
            if responses.is_err() {
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

    #[cfg(not(feature = "complete-compare"))]
    async fn compare_accounts(&self, block: U64, timestamp: U256) -> Result<()> {
        let batch = 100;
        let mut success = true;

        let mut multicall_pool = Vec::new();
        for _ in 0..(self.accounts.len() - 1 + batch) / batch {
            multicall_pool.push(
                Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None).await?,
            );
        }
        self.accounts
            .iter()
            .enumerate()
            .for_each(|(i, (account, _))| {
                let new_position = i / batch;
                multicall_pool[new_position].add_call(
                    self.auditor
                        .account_liquidity(*account, Address::zero(), U256::zero()),
                    false,
                );
            });

        if !self.accounts.is_empty() {
            let mut tasks = Vec::new();
            for multicall in multicall_pool {
                tasks.push(tokio::spawn(async move {
                    multicall
                        .version(MulticallVersion::Multicall)
                        .block(block)
                        .call_raw()
                        .await
                }));
            }
            let mut responses = Vec::new();
            for task in tasks {
                responses.append(&mut task.await.unwrap()?);
            }
            for (token, (address, _)) in responses.iter().zip(&self.accounts) {
                let v = token.clone().into_tokens();
                let (adjusted_collateral, adjusted_debt): (U256, U256) = (
                    v[0].clone().into_uint().unwrap(),
                    v[1].clone().into_uint().unwrap(),
                );
                if adjusted_debt > U256::zero() {
                    let previewer_hf = adjusted_collateral.div_wad_down(adjusted_debt);
                    if let Some(account) = self.accounts.get(address) {
                        let hf = Self::compute_hf(&self.markets, account, timestamp).map(
                            |(hf, collateral, debt, _)| {
                                success &=
                                    compare!("health_factor", &account, "", previewer_hf, hf);
                                success &= compare!(
                                    "collateral",
                                    &account,
                                    "",
                                    adjusted_collateral,
                                    collateral
                                );
                                success &= compare!("debt", &account, "", adjusted_debt, debt);
                                hf
                            },
                        );
                        println!(
                            "{:<20?}: {:>20}{}",
                            address,
                            hf.as_ref().unwrap_or(&previewer_hf),
                            if hf.is_err() { "*" } else { "" }
                        );
                    }
                }
            }
        }
        if !success {
            panic!("compare accounts error");
        }
        Ok(())
    }

    #[cfg(feature = "complete-compare")]
    async fn compare_accounts(&self, block: U64, timestamp: U256) -> Result<()> {
        let previewer_accounts = &self.multicall_previewer(U64::from(&block)).await;
        let accounts = &self.accounts;
        let mut success = true;
        let mut compare_markets = true;
        let mut accounts_with_borrows = Vec::<Address>::new();

        if previewer_accounts.is_empty() {
            return Ok(());
        };
        for (address, previewer_account) in previewer_accounts {
            println!("Comparing account {:#?}", address);
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
                for market in self.markets.values() {
                    if market.price_feed.is_some() && !market.base_market {
                        multicall.add_call(
                            self.auditor
                                .asset_price(market.price_feed.as_ref().unwrap().address),
                            true,
                        );
                    }
                }
                let responses = multicall
                    .version(MulticallVersion::Multicall)
                    .block(block)
                    .call_raw()
                    .await?;
                self.markets
                    .values()
                    .filter(|market| market.price_feed.is_some() && !market.base_market)
                    .zip(&responses)
                    .for_each(|(market, response)| {
                        success &= compare!(
                            "price",
                            "",
                            format!("{:#?}/{:#?}", market.contract.address(), market.price_feed),
                            response.clone().into_uint().unwrap(),
                            market.price
                        );
                    });

                let mut multicall =
                    Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None)
                        .await?;
                const FIELDS: usize = 9;
                for market in self.markets.values() {
                    multicall.add_call(market.contract.floating_assets(), false);
                    multicall.add_call(market.contract.last_accumulator_accrual(), false);
                    multicall.add_call(market.contract.earnings_accumulator(), false);
                    multicall.add_call(market.contract.earnings_accumulator_smooth_factor(), false);
                    multicall.add_call(market.contract.floating_debt(), false);
                    multicall.add_call(market.contract.total_floating_borrow_shares(), false);
                    multicall.add_call(market.contract.last_floating_debt_update(), false);
                    multicall.add_call(market.contract.floating_utilization(), false);
                    multicall.add_call(market.contract.total_supply(), false);
                }
                let responses = multicall
                    .version(MulticallVersion::Multicall)
                    .block(block)
                    .call_raw()
                    .await?;
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
                        market.earnings_accumulator_smooth_factor
                    );
                    success &= compare!(
                        "floating_debt",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 4].clone().into_uint().unwrap(),
                        market.floating_debt
                    );
                    success &= compare!(
                        "total_floating_borrow_shares",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 5].clone().into_uint().unwrap(),
                        market.floating_borrow_shares
                    );
                    success &= compare!(
                        "total_floating_borrow_shares",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 5].clone().into_uint().unwrap(),
                        market.floating_borrow_shares
                    );
                    success &= compare!(
                        "last_floating_debt_update",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 6].clone().into_uint().unwrap(),
                        market.last_floating_debt_update
                    );
                    success &= compare!(
                        "floating_utilization",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 7].clone().into_uint().unwrap(),
                        market.floating_utilization
                    );
                    success &= compare!(
                        "total_floating_deposit_shares",
                        "",
                        previewer_market.market,
                        responses[i * FIELDS + 8].clone().into_uint().unwrap(),
                        market.floating_deposit_shares
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
                    "floating_deposit_assets",
                    address,
                    market_account.market,
                    market_account.floating_deposit_assets,
                    account
                        .floating_deposit_assets(&self.markets[&market_account.market], timestamp)
                );
                success &= compare!(
                    "floating_borrow_assets",
                    address,
                    market_account.market,
                    market_account.floating_borrow_assets,
                    account
                        .floating_borrow_assets(&self.markets[&market_account.market], timestamp)
                );
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
            multicall.add_call(
                self.auditor
                    .account_liquidity(*account, Address::zero(), U256::zero()),
                false,
            );
        }

        let responses = multicall
            .version(MulticallVersion::Multicall)
            .block(block)
            .call_raw()
            .await?;

        for (token, account) in responses.iter().zip(accounts_with_borrows) {
            let v = token.clone().into_tokens();
            let (adjusted_collateral, adjusted_debt): (U256, U256) = (
                v[0].clone().into_uint().unwrap(),
                v[1].clone().into_uint().unwrap(),
            );
            println!("account            : {}", account);
            println!("adjusted collateral: {}", adjusted_collateral);
            println!("adjusted debt      : {}", adjusted_debt);
            if adjusted_debt > U256::zero() {
                let previewer_hf = adjusted_collateral.div_wad_down(adjusted_debt);
                println!("health factor      : {}", previewer_hf);
                if let Some(account) = self.accounts.get(&account) {
                    if let Ok((hf, collateral, debt, _)) =
                        Self::compute_hf(&self.markets, account, timestamp)
                    {
                        success &= compare!("health_factor", &account, "", previewer_hf, hf);
                        success &=
                            compare!("collateral", &account, "", adjusted_collateral, collateral);
                        success &= compare!("debt", &account, "", adjusted_debt, debt);
                    }
                }
            } else {
                println!(
                    "Account: {} collateral: {} debt: {}",
                    &account, adjusted_collateral, adjusted_debt
                );
                println!("***************");
                println!("Account: {:#?}", self.accounts[&account]);
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
                if let Some(receipt) = abi.get("receipt") {
                    receipt.clone()
                } else {
                    Value::Null
                },
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
        let abi: Abi = serde_json::from_value(abi).unwrap();
        let block_number = if let Value::Object(receipt) = receipt {
            if let Value::Number(block_number) = &receipt["blockNumber"] {
                block_number.as_u64().unwrap()
            } else {
                panic!("Invalid ABI")
            }
        } else {
            0
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
                    .mul_div_down(market.price, U256::exp10(market.decimals as usize));
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
                if *maturity < timestamp {
                    current_debt +=
                        borrowed.mul_wad_down((timestamp - *maturity) * market.penalty_rate)
                }
            }
            current_debt += position.floating_borrow_assets(market, timestamp);

            let value =
                current_debt.mul_div_up(market.price, U256::exp10(market.decimals as usize));
            total_debt += value;
            adjusted_debt += value.div_wad_up(market.adjust_factor);
            if value >= repay.market_to_liquidate_debt {
                repay.price = market.price;
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
