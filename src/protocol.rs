use super::config::Config;
use super::exactly_events::ExactlyEvents;
use super::fixed_point_math::{math, FixedPointMath, FixedPointMathGen};
use super::liquidation::{Liquidation, LiquidationData, Repay};
use crate::generate_abi::interest_rate_model::InterestRateModel;
use crate::generate_abi::{
    AggregatorProxy, Auditor, LiquidationIncentive, Previewer, PriceFeed, PriceFeedDouble,
    PriceFeedLido, PriceFeedWrapper,
};
use crate::liquidation::{self, LiquidationAction, ProtocolState};
use crate::market::{PriceDouble, PriceFeedController, PriceFeedType, PriceRate};
use crate::network::{Network, NetworkStatus};
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
use ethers::types::H256;
use eyre::{eyre, Report, Result};
use log::{info, warn};
use sentry::{add_breadcrumb, Breadcrumb, Level};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::{collections::HashMap, time::Duration};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::Mutex;
use tokio::time;
use tokio_stream::StreamExt;

#[cfg(feature = "complete-compare")]
use crate::generate_abi::MarketAccount;

#[cfg(feature = "liquidation-stats")]
use crate::generate_abi::LiquidateFilter;

pub const BASE_FEED: &str = "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE";

const CACHE_PATH: &str = "cache";
const CACHE_PROTOCOL: &str = "protocol";

pub type TokenPairFeeMap = HashMap<(Address, Address), BinaryHeap<Reverse<u32>>>;

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug, Serialize, Deserialize)]
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
    Finish,
}

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug, Serialize, Deserialize)]
struct ContractKey {
    address: Address,
    kind: ContractKeyKind,
    index: usize,
}

macro_rules! compare {
    ($label:expr, $account:expr, $market:expr, $ref:expr, $val:expr $(, $( $print:expr$(,)? )?)?) => {
        if $ref != $val {
            $($( if $print == true {
                warn!("{:?}@{}.{}", $account, $market, $label);
                warn!("reference: {:#?}", $ref);
                warn!("    value: {:#?}", $val);
            } )? )?
            false
        } else {
            true
        }
    };
}

// #[derive(Debug)]
#[derive(Serialize, Deserialize, Default, Debug)]
struct ProtocolData {
    pub last_sync: (U64, i128, i128),
    pub markets: HashMap<Address, Market>,
    pub sp_fee_rate: U256,
    pub accounts: HashMap<Address, Account>,
    #[serde(with = "hashmap_as_vector")]
    pub contracts_to_listen: HashMap<ContractKey, Address>,
    pub comparison_enabled: bool,
    pub liquidation_incentive: LiquidationIncentive,
    pub market_weth_address: Address,
    pub price_decimals: U256,
    pub repay_offset: U256,
}

impl ProtocolData {
    fn cache_path(chain_id: u64) -> String {
        [CACHE_PATH, chain_id.to_string().as_str()]
            .iter()
            .collect::<PathBuf>()
            .to_str()
            .unwrap_or(CACHE_PATH)
            .to_string()
    }

    pub async fn from_cache_or_new<M: 'static + Middleware, S: 'static + Signer>(
        auditor: &Auditor<SignerMiddleware<M, S>>,
        deployed_block: u64,
        market_weth_address: Address,
        config: &Config,
    ) -> Result<ProtocolData> {
        let protocol_data =
            match cacache::read(ProtocolData::cache_path(config.chain_id), CACHE_PROTOCOL).await {
                Ok(cache) => {
                    if let Ok(protocol_data) = serde_json::from_slice::<ProtocolData>(&cache) {
                        protocol_data
                    } else {
                        Self::new(auditor, deployed_block, config, market_weth_address).await?
                    }
                }
                Err(e) => {
                    warn!("Failed to load protocol data from cache: {}", e);
                    Self::new(auditor, deployed_block, config, market_weth_address).await?
                }
            };
        Ok(protocol_data)
    }
    async fn new<M: 'static + Middleware, S: 'static + Signer>(
        auditor: &Auditor<SignerMiddleware<M, S>>,
        deployed_block: u64,
        config: &Config,
        market_weth_address: ethers::types::H160,
    ) -> Result<ProtocolData, eyre::ErrReport> {
        let auditor_markets = auditor.all_markets().call().await?;
        let mut markets = HashMap::<Address, Market>::new();
        for market in auditor_markets {
            markets
                .entry(market)
                .or_insert_with_key(|key| Market::new(*key));
        }
        let mut contracts_to_listen = HashMap::new();
        contracts_to_listen.insert(
            ContractKey {
                address: (*auditor).address(),
                kind: ContractKeyKind::Auditor,
                index: 0,
            },
            (*auditor).address(),
        );
        Ok(ProtocolData {
            last_sync: (U64::from(deployed_block), -1, -1),
            markets,
            sp_fee_rate: U256::zero(),
            accounts: HashMap::new(),
            contracts_to_listen,
            comparison_enabled: config.comparison_enabled,
            liquidation_incentive: Default::default(),
            market_weth_address,
            price_decimals: U256::zero(),
            repay_offset: config.repay_offset,
        })
    }
}

pub struct Protocol<M, W, S> {
    client: Arc<SignerMiddleware<M, S>>,
    previewer: Previewer<SignerMiddleware<M, S>>,
    auditor: Auditor<SignerMiddleware<M, S>>,
    liquidation: Arc<Mutex<Liquidation<M, W, S>>>,
    liquidation_sender: Sender<LiquidationData>,
    multicall: Multicall<SignerMiddleware<M, S>>,
    token_pairs: Arc<TokenPairFeeMap>,
    tokens: Arc<HashSet<Address>>,
    wait_for_final_event: bool,
    data: ProtocolData,
    chain_id: u64,
    config: Config,
    network: Arc<Network>,
}

impl<
        M: 'static + Middleware + Clone,
        W: 'static + Middleware + Clone,
        S: 'static + Signer + Clone,
    > std::fmt::Debug for Protocol<M, W, S>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Protocol").finish()
    }
}

#[derive(Debug)]
pub enum ProtocolError<M: 'static + Middleware, S: 'static + Signer> {
    SignerMiddlewareError(SignerMiddlewareError<M, S>),
    EthError,
    Other(String),
}

impl<
        M: 'static + Middleware + Clone,
        W: 'static + Middleware + Clone,
        S: 'static + Signer + Clone,
    > Protocol<M, W, S>
{
    async fn get_contracts(
        client: Arc<SignerMiddleware<M, S>>,
        config: &Config,
    ) -> (
        u64,
        Auditor<SignerMiddleware<M, S>>,
        Previewer<SignerMiddleware<M, S>>,
    ) {
        let (auditor_address, _, deployed_block) = Protocol::<M, W, S>::parse_abi(&format!(
            "node_modules/@exactly/protocol/deployments/{}/Auditor.json",
            config.chain_id_name
        ));

        let (previewer_address, _, _) = Protocol::<M, W, S>::parse_abi(&format!(
            "node_modules/@exactly/protocol/deployments/{}/Previewer.json",
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

        let (market_weth_address, _, _) = Self::parse_abi(&format!(
            "node_modules/@exactly/protocol/deployments/{}/MarketWETH.json",
            config.chain_id_name
        ));

        let network = Arc::new(Network::from_config(config));

        let (liquidation_sender, liquidation_receiver) = mpsc::channel(1);

        let liquidation = Arc::new(Mutex::new(Liquidation::new(
            Arc::clone(&client),
            Arc::clone(&client_relayer),
            &config.token_pairs,
            previewer.clone(),
            auditor.clone(),
            market_weth_address,
            config,
            Arc::clone(&network),
        )));

        let liquidation_lock = liquidation.lock().await;
        let tokens = liquidation_lock.get_tokens();
        let token_pairs = liquidation_lock.get_token_pairs();
        drop(liquidation_lock);

        let liquidation_clone = Arc::clone(&liquidation);
        tokio::spawn(async move {
            let _ = Liquidation::run(liquidation_clone, liquidation_receiver).await;
        });

        let data =
            ProtocolData::from_cache_or_new(&auditor, deployed_block, market_weth_address, config)
                .await?;
        let multicall = Multicall::new(Arc::clone(&client), None).await?;

        Ok(Protocol {
            client: Arc::clone(&client),
            auditor,
            previewer,
            liquidation,
            liquidation_sender,
            multicall,
            tokens,
            token_pairs,
            wait_for_final_event: false,
            data,
            chain_id: config.chain_id,
            config: config.clone(),
            network: Arc::clone(&network),
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
    }

    async fn debounce_liquidations(
        service: Arc<Mutex<Self>>,
        mut debounce_rx: Receiver<TaskActivity>,
    ) {
        let mut block_number = None;
        let mut check_liquidations = false;
        let mut liquidation_checked = false;
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
                    TaskActivity::Finish => break,
                },
                Ok(None) => break,
                Err(_) => {
                    if check_liquidations && !liquidation_checked {
                        if let Some(block) = block_number {
                            info!("Locking before check liquidations for block {}", block);
                            let lock = service.lock().await.check_liquidations(block, user).await;
                            liquidation_checked = true;
                            drop(lock);
                        }
                    }
                }
            }
        }
    }

    pub async fn launch(service: Arc<Mutex<Self>>) -> Result<(), ProtocolError<M, S>>
    where
        <M as Middleware>::Provider: PubsubClient,
    {
        let page_size: u64 = service.lock().await.config.page_size;

        let (debounce_tx, debounce_rx) = mpsc::channel(10);
        let debounce_liquidation = tokio::spawn(Self::debounce_liquidations(
            Arc::clone(&service),
            debounce_rx,
        ));
        enum DataFrom<'a, M: Middleware, S: Signer>
        where
            <M as Middleware>::Provider: PubsubClient,
        {
            Log(Vec<Log>),
            Stream(Result<SubscriptionStream<'a, M::Provider, Log>, SignerMiddlewareError<M, S>>),
        }
        let file = File::create("data.log").unwrap();
        let mut writer = BufWriter::new(file);
        let mut first_block = service.lock().await.data.last_sync.0;
        let mut last_block = first_block + page_size;
        let mut latest_block = U64::zero();
        let mut getting_logs = true;
        'filter: loop {
            let client;
            let result: DataFrom<M, S> = {
                let service_unlocked = service.lock().await;
                client = Arc::clone(&service_unlocked.client);
                if latest_block.is_zero() {
                    let gbn = client.get_block_number().await;
                    latest_block = gbn.unwrap_or(service_unlocked.data.last_sync.0);
                }

                let mut filter = Filter::new().from_block(first_block).address(
                    service_unlocked
                        .data
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
                        break 'filter;
                    }
                } else {
                    DataFrom::Stream(client.subscribe_logs(&filter).await)
                }
            };
            match result {
                DataFrom::Log(logs) => {
                    _ = debounce_tx.send(TaskActivity::StopCheckLiquidation).await;
                    info!("locking service to handle logs");
                    let mut me = service.lock().await;
                    info!("handling logs");
                    for log in logs {
                        let status = me.handle_log(log, &debounce_tx, &mut writer).await;
                        match status {
                            Ok(result) => {
                                if let LogIterating::UpdateFilters = result {
                                    first_block = me.data.last_sync.0;
                                    continue 'filter;
                                }
                            }
                            Err(e) => {
                                protocol_error_breadcrumb(&e);
                                panic!("Error handling log {:?}", e);
                            }
                        };
                    }
                    info!("logs handled");
                    if last_block >= latest_block {
                        first_block = latest_block;
                        getting_logs = false;
                        let _ = debounce_tx
                            .send(TaskActivity::UpdateAll(Some(latest_block)))
                            .await;
                        let _ = debounce_tx.send(TaskActivity::StartCheckLiquidation).await;
                        let backup = serde_json::to_vec(&me.data).unwrap();
                        info!("writing cache");
                        match cacache::write(
                            ProtocolData::cache_path(me.chain_id),
                            CACHE_PROTOCOL,
                            &backup,
                        )
                        .await
                        {
                            Ok(_) => info!("cache written"),
                            Err(e) => info!("error writing cache {:?}", e),
                        }
                        info!("cache written");
                    } else {
                        info!("getting next page");
                        first_block = last_block + 1u64;
                        last_block = if first_block + page_size > latest_block {
                            latest_block
                        } else {
                            last_block + page_size
                        };
                    }
                }
                DataFrom::Stream(stream) => match stream {
                    Ok(mut stream) => {
                        _ = std::io::Write::flush(&mut writer);
                        while let Some(log) = stream.next().await {
                            let mut me = service.lock().await;
                            let status = me.handle_log(log, &debounce_tx, &mut writer).await;
                            match status {
                                Ok(result) => {
                                    if let LogIterating::UpdateFilters = result {
                                        _ = debounce_tx
                                            .send(TaskActivity::StopCheckLiquidation)
                                            .await;
                                        first_block = me.data.last_sync.0;
                                        continue 'filter;
                                    } else {
                                        if me.wait_for_final_event {
                                            _ = debounce_tx
                                                .send(TaskActivity::StopCheckLiquidation)
                                                .await;
                                        } else {
                                            _ = debounce_tx
                                                .send(TaskActivity::StartCheckLiquidation)
                                                .await;
                                        }
                                        let backup = serde_json::to_vec(&me.data).unwrap();
                                        cacache::write(
                                            ProtocolData::cache_path(me.chain_id),
                                            CACHE_PROTOCOL,
                                            &backup,
                                        )
                                        .await
                                        .unwrap();
                                    }
                                }
                                Err(e) => {
                                    protocol_error_breadcrumb(&e);
                                    panic!("Protocol error");
                                }
                            };
                        }
                        let data = BTreeMap::new();
                        sentry::add_breadcrumb(Breadcrumb {
                            ty: "error".to_string(),
                            category: Some("Stream closed".to_string()),
                            level: Level::Error,
                            data,
                            ..Default::default()
                        });
                    }
                    Err(e) => {
                        let mut data = BTreeMap::new();
                        data.insert(
                            "error message".to_string(),
                            Value::String(format!("{:?}", e)),
                        );
                        if let SignerMiddlewareError::MiddlewareError(m) = &e {
                            data.insert(
                                "error message - middleware".to_string(),
                                Value::String(format!("{:?}", m)),
                            );
                        }
                        sentry::add_breadcrumb(Breadcrumb {
                            ty: "error".to_string(),
                            category: Some("Connection error".to_string()),
                            level: Level::Error,
                            data,
                            ..Default::default()
                        });
                        if debounce_tx.send(TaskActivity::Finish).await.is_err() {
                            debounce_liquidation.abort();
                        }
                        panic!("Connection error");
                    }
                },
            };
        }
        if debounce_tx.send(TaskActivity::Finish).await.is_err() {
            debounce_liquidation.abort();
        }
        Ok(())
    }

    async fn update_price_lido(
        price_feed: &mut PriceFeedController,
        block: U64,
        client: Arc<SignerMiddleware<M, S>>,
        price_decimals: U256,
    ) -> Result<U256> {
        let main_price: U256 = if let Some(main_price_feed) = &mut price_feed.main_price_feed {
            let contract = PriceFeed::new(main_price_feed.address, Arc::clone(&client));
            contract
                .latest_answer()
                .block(block)
                .call()
                .await?
                .into_raw()
        } else {
            U256::zero()
        };
        if let Some(PriceFeedType::Single(wrapper)) = price_feed.wrapper.as_mut() {
            let price_feed_contract = PriceFeedLido::new(wrapper.address, Arc::clone(&client));
            let conversion_selector_method: ContractCall<SignerMiddleware<M, S>, U256> =
                price_feed_contract.method_hash(wrapper.conversion_selector, wrapper.base_unit)?;
            let rate: U256 = conversion_selector_method.block(block).call().await?;
            wrapper.rate = rate;
            wrapper.main_price = main_price;
            return Ok(rate.mul_div_down(main_price, wrapper.base_unit)
                * U256::exp10(18 - price_decimals.as_usize()));
        }
        Err(eyre!("Price feed not found"))
    }

    async fn update_prices(&mut self, on_market: Address, block: U64) -> Result<()> {
        let price_feed: Vec<PriceFeed<SignerMiddleware<M, S>>> = self
            .data
            .markets
            .iter()
            .filter(|(k, v)| {
                **k == on_market && v.listed && v.price_feed.is_some() && !v.base_market
            })
            .map(|(_, market)| {
                PriceFeed::new(
                    market.price_feed.as_ref().unwrap().address,
                    self.client.clone(),
                )
            })
            .collect();
        if price_feed.is_empty() {
            return Ok(());
        }
        let mut multicall = self.multicall.clone();
        multicall.clear_calls();
        price_feed.iter().for_each(|price_feed_wrapper| {
            multicall.add_call(price_feed_wrapper.latest_answer(), false);
        });
        let results = multicall
            .version(MulticallVersion::Multicall)
            .block(block)
            .call_raw()
            .await?;

        for ((_, market), result) in self
            .data
            .markets
            .iter_mut()
            .filter(|(k, m)| {
                **k == on_market && m.listed && m.price_feed.is_some() && !m.base_market
            })
            .zip(results)
        {
            let value = result
                .clone()
                .map_err(|_| eyre!("wrong value"))?
                .into_int()
                .unwrap();
            let price = if let Some(wrapper_type) =
                market.price_feed.as_mut().and_then(|p| p.wrapper).as_mut()
            {
                match wrapper_type {
                    PriceFeedType::Single(_) => {
                        Self::update_price_lido(
                            market.price_feed.as_mut().unwrap(),
                            block,
                            Arc::clone(&self.client),
                            self.data.price_decimals,
                        )
                        .await?
                    }
                    PriceFeedType::Double(wrapper) => {
                        let price_feed_one =
                            PriceFeed::new(wrapper.price_feed_one, self.client.clone());
                        let price_feed_two =
                            PriceFeed::new(wrapper.price_feed_two, self.client.clone());
                        let mut multicall = self.multicall.clone();
                        multicall.clear_calls();
                        multicall.add_call(price_feed_one.latest_answer(), false);
                        multicall.add_call(price_feed_two.latest_answer(), false);
                        let results: (U256, U256) = multicall
                            .version(MulticallVersion::Multicall)
                            .block(block)
                            .call()
                            .await?;
                        if let PriceFeedType::Double(w) = market
                            .price_feed
                            .as_mut()
                            .unwrap()
                            .wrapper
                            .as_mut()
                            .unwrap()
                        {
                            (w.price_one, w.price_two) = results;
                        }
                        (wrapper.price_one, wrapper.price_two) = results;
                        wrapper
                            .price_one
                            .mul_div_down(wrapper.price_two, wrapper.base_unit)
                            * U256::exp10(18 - self.data.price_decimals.as_usize())
                    }
                }
            } else {
                value * U256::exp10(18 - self.data.price_decimals.as_usize())
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
        let topics = log.topics.clone();
        let log_data = log.data.to_vec();
        let result = ExactlyEvents::decode_log(&RawLog {
            topics: log.topics,
            data: log.data.to_vec(),
        });
        if result.is_err() {
            warn!("{:?}", meta);
        }
        let event = result?;

        if (
            meta.block_number,
            meta.transaction_index.as_u64() as i128,
            meta.log_index.as_u128() as i128,
        ) <= self.data.last_sync
        {
            return Ok(LogIterating::NextLog);
        }
        self.data.last_sync = (
            meta.block_number,
            meta.transaction_index.as_u64() as i128,
            meta.log_index.as_u128() as i128,
        );
        sentry_breadcrumb(&meta, topics, log_data, &event);
        match event {
            ExactlyEvents::MaxFuturePoolsSet(data) => {
                self.data
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| Market::new(*key))
                    .max_future_pools = data.max_future_pools.as_u32() as u8;
            }

            ExactlyEvents::EarningsAccumulatorSmoothFactorSet(data) => {
                self.data
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| Market::new(*key))
                    .earnings_accumulator_smooth_factor = data.earnings_accumulator_smooth_factor;
            }

            ExactlyEvents::MarketListed(data) => {
                let mut market = self
                    .data
                    .markets
                    .entry(data.market)
                    .or_insert_with_key(|key| Market::new(*key));
                market.decimals = data.decimals;
                market.smart_pool_fee_rate = self.data.sp_fee_rate;
                market.listed = true;
                let contract = market.contract(Arc::clone(&self.client));
                let mut multicall = self.multicall.clone();
                multicall.clear_calls();
                multicall.add_call(contract.max_future_pools(), false);
                multicall.add_call(contract.earnings_accumulator_smooth_factor(), false);
                multicall.add_call(contract.interest_rate_model(), false);
                multicall.add_call(contract.penalty_rate(), false);
                multicall.add_call(contract.treasury_fee_rate(), false);
                multicall.add_call(contract.asset(), false);
                multicall.add_call(contract.last_accumulator_accrual(), false);
                multicall.add_call(contract.last_floating_debt_update(), false);
                multicall.add_call(contract.symbol(), false);
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
                    symbol,
                ) = multicall.call().await?;
                market.max_future_pools = max_future_pools;
                market.earnings_accumulator_smooth_factor = earnings_accumulator_smooth_factor;
                market.interest_rate_model = interest_rate_model;
                market.penalty_rate = penalty_rate;
                market.treasury_fee_rate = treasury_fee_rate;
                market.asset = asset;
                market.last_accumulator_accrual = last_accumulator_accrual;
                market.last_floating_debt_update = last_floating_debt_update;
                market.symbol = symbol;
                let irm =
                    InterestRateModel::new(market.interest_rate_model, Arc::clone(&self.client));
                multicall.clear_calls();
                multicall.add_call(irm.floating_curve_a(), false);
                multicall.add_call(irm.floating_curve_b(), false);
                multicall.add_call(irm.floating_max_utilization(), false);
                multicall = multicall
                    .version(MulticallVersion::Multicall)
                    .block(meta.block_number);
                let result = multicall.call().await;
                if let Ok((floating_a, floating_b, floating_max_utilization)) = result {
                    market.floating_a = floating_a;
                    market.floating_b = floating_b;
                    market.floating_max_utilization = floating_max_utilization;
                }
                self.data.contracts_to_listen.insert(
                    ContractKey {
                        address: data.market,
                        kind: ContractKeyKind::Market,
                        index: 0,
                    },
                    data.market,
                );
                self.update_prices(data.market, meta.block_number).await?;
                self.data.last_sync = (
                    meta.block_number,
                    meta.transaction_index.as_u64() as i128,
                    meta.log_index.as_u128() as i128,
                );
                return Ok(LogIterating::UpdateFilters);
            }

            ExactlyEvents::InterestRateModelSet(data) => {
                let mut market = self.data.markets.get_mut(&meta.address).unwrap();
                market.interest_rate_model = data.interest_rate_model;
                let irm =
                    InterestRateModel::new(market.interest_rate_model, Arc::clone(&self.client));
                let mut multicall = self.multicall.clone();
                multicall.clear_calls();
                multicall.add_call(irm.floating_curve_a(), false);
                multicall.add_call(irm.floating_curve_b(), false);
                multicall.add_call(irm.floating_max_utilization(), false);
                multicall = multicall
                    .version(MulticallVersion::Multicall)
                    .block(meta.block_number);
                let result = multicall.call().await;
                if let Ok((floating_a, floating_b, floating_max_utilization)) = result {
                    market.floating_a = floating_a;
                    market.floating_b = floating_b;
                    market.floating_max_utilization = floating_max_utilization;
                }
                self.data.last_sync = (
                    meta.block_number,
                    meta.transaction_index.as_u64() as i128,
                    meta.log_index.as_u128() as i128,
                );
                return Ok(LogIterating::UpdateFilters);
            }

            ExactlyEvents::Transfer(data) => {
                if data.to != Address::zero() {
                    self.data
                        .accounts
                        .entry(data.to)
                        .or_insert_with_key(|key| Account::new(*key, &self.data.markets))
                        .positions
                        .entry(meta.address)
                        .or_default()
                        .floating_deposit_shares += data.amount;
                }
                if data.from != Address::zero() {
                    self.data
                        .accounts
                        .get_mut(&data.from)
                        .unwrap()
                        .positions
                        .get_mut(&meta.address)
                        .unwrap()
                        .floating_deposit_shares -= data.amount;
                }
            }
            ExactlyEvents::Borrow(data) => {
                self.data
                    .accounts
                    .entry(data.borrower)
                    .or_insert_with_key(|key| Account::new(*key, &self.data.markets))
                    .positions
                    .entry(meta.address)
                    .or_default()
                    .floating_borrow_shares += data.shares;
            }
            ExactlyEvents::Repay(data) => {
                self.data
                    .accounts
                    .entry(data.borrower)
                    .or_insert_with_key(|key| Account::new(*key, &self.data.markets))
                    .positions
                    .entry(meta.address)
                    .or_default()
                    .floating_borrow_shares -= data.shares;
            }
            ExactlyEvents::DepositAtMaturity(data) => {
                self.data
                    .accounts
                    .entry(data.owner)
                    .or_insert_with(|| Account::new(data.owner, &self.data.markets))
                    .deposit_at_maturity(&data, &meta.address);
            }
            ExactlyEvents::WithdrawAtMaturity(data) => {
                self.data
                    .accounts
                    .entry(data.owner)
                    .or_insert_with(|| Account::new(data.owner, &self.data.markets))
                    .withdraw_at_maturity(data, &meta.address);
            }
            ExactlyEvents::BorrowAtMaturity(data) => {
                self.data
                    .accounts
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.data.markets))
                    .borrow_at_maturity(&data, &meta.address);
            }
            ExactlyEvents::RepayAtMaturity(data) => {
                self.data
                    .accounts
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.data.markets))
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

            ExactlyEvents::AdjustFactorSet(data) => {
                self.data
                    .markets
                    .entry(data.market)
                    .or_insert_with_key(|key| Market::new(*key))
                    .adjust_factor = data.adjust_factor;
            }

            ExactlyEvents::PenaltyRateSet(data) => {
                self.data
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| Market::new(*key))
                    .penalty_rate = data.penalty_rate;
            }

            ExactlyEvents::MarketEntered(data) => {
                self.data
                    .accounts
                    .entry(data.account)
                    .or_insert_with(|| Account::new(data.account, &self.data.markets))
                    .set_collateral(&data.market);
            }

            ExactlyEvents::MarketExited(data) => {
                self.data
                    .accounts
                    .entry(data.account)
                    .or_insert_with(|| Account::new(data.account, &self.data.markets))
                    .unset_collateral(&data.market);
            }

            ExactlyEvents::BackupFeeRateSet(data) => {
                self.data.sp_fee_rate = data.backup_fee_rate;
                for market in self.data.markets.values_mut() {
                    market.smart_pool_fee_rate = data.backup_fee_rate;
                }
            }

            ExactlyEvents::PriceFeedSetFilter(data) => {
                if data.price_feed == Address::from_str(BASE_FEED).unwrap() {
                    let market = self
                        .data
                        .markets
                        .entry(data.market)
                        .or_insert_with_key(|key| Market::new(*key));

                    market.price_feed =
                        Some(PriceFeedController::main_price_feed(data.price_feed, None));
                    market.price = U256::exp10(self.data.price_decimals.as_usize());
                    market.base_market = true;
                    return Ok(LogIterating::NextLog);
                }
                let on_market = data.market;
                self.handle_price_feed(data, &meta).await?;
                self.update_prices(on_market, meta.block_number).await?;
                self.data.last_sync = (
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
                // Comparison enabled again after NewTransmission event
                self.wait_for_final_event = false;
                self.data.markets.iter_mut().for_each(|(_, market)| {
                    if let Some(price_feed) = market
                        .price_feed
                        .as_mut()
                        .filter(|p| p.event_emitters.contains(&meta.address))
                    {
                        let price = if let Some(PriceFeedType::Single(wrapper)) =
                            price_feed.wrapper.as_mut()
                        {
                            wrapper.main_price = data.current.into_raw();
                            wrapper
                                .rate
                                .mul_div_down(wrapper.main_price, wrapper.base_unit)
                                * U256::exp10(18 - self.data.price_decimals.as_usize())
                        } else if let Some(PriceFeedType::Double(wrapper)) =
                            price_feed.wrapper.as_mut()
                        {
                            if meta.address == price_feed.event_emitters[0] {
                                wrapper.price_one = data.current.into_raw();
                            } else if meta.address == price_feed.event_emitters[1] {
                                wrapper.price_two = data.current.into_raw();
                            }
                            wrapper
                                .price_one
                                .mul_div_down(wrapper.price_two, wrapper.base_unit)
                                * U256::exp10(18 - self.data.price_decimals.as_usize())
                        } else {
                            data.current.into_raw()
                                * U256::exp10(18 - self.data.price_decimals.as_usize())
                        };
                        market.price = price;
                    };
                });
            }

            ExactlyEvents::MarketUpdate(data) => {
                let market = self
                    .data
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address));
                market.floating_deposit_shares = data.floating_deposit_shares;
                market.floating_assets = data.floating_assets;
                market.earnings_accumulator = data.earnings_accumulator;
                market.floating_debt = data.floating_debt;
                market.floating_borrow_shares = data.floating_borrow_shares;
            }

            ExactlyEvents::FloatingDebtUpdate(data) => {
                let market = self
                    .data
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address));
                market.floating_utilization = data.utilization;
                market.last_floating_debt_update = data.timestamp;
            }

            ExactlyEvents::AccumulatorAccrual(data) => {
                let market = self
                    .data
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address));
                market.last_accumulator_accrual = data.timestamp;
            }

            ExactlyEvents::FixedEarningsUpdate(data) => {
                let market = self
                    .data
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address));

                let pool = market.fixed_pools.entry(data.maturity).or_default();
                pool.last_accrual = data.timestamp;
                pool.unassigned_earnings = data.unassigned_earnings;
            }

            ExactlyEvents::TreasurySet(data) => {
                let market = self
                    .data
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| Market::new(*address));
                market.treasury_fee_rate = data.treasury_fee_rate;
            }

            ExactlyEvents::LiquidationIncentiveSet(data) => {
                self.data.liquidation_incentive = data.liquidation_incentive;
            }

            ExactlyEvents::Initialized(_) => {
                self.data.price_decimals = self
                    .auditor
                    .price_decimals()
                    .block(meta.block_number)
                    .call()
                    .await?;
            }

            ExactlyEvents::PostTotalShares(data) => {
                self.data
                    .markets
                    .iter_mut()
                    .filter(|(_, market)| {
                        if let Some(price_feed) = market.price_feed.as_ref() {
                            price_feed.wrapper.is_some()
                        } else {
                            false
                        }
                    })
                    .for_each(|(_, market)| {
                        let wrapper_type = market
                            .price_feed
                            .as_mut()
                            .unwrap()
                            .wrapper
                            .as_mut()
                            .unwrap();
                        if let PriceFeedType::Single(wrapper) = wrapper_type {
                            wrapper.rate = if data.total_shares == 0.into() {
                                U256::zero()
                            } else {
                                wrapper.base_unit * data.post_total_pooled_ether / data.total_shares
                            };
                            market.price = wrapper
                                .rate
                                .mul_div_down(wrapper.main_price, wrapper.base_unit)
                                * U256::exp10(18 - self.data.price_decimals.as_usize());
                        }
                    });
            }
            ExactlyEvents::NewTransmission(_) => {
                // Enable comparison just after receive AnswerUpdated event
                self.wait_for_final_event = true;
            }
            _ => {}
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
        let double = PriceFeedDouble::new(data.price_feed, Arc::clone(&self.client));
        let mut wrapper_single_multicall = self.multicall.clone();
        wrapper_single_multicall.clear_calls();
        wrapper_single_multicall.add_call(wrapper.main_price_feed(), true);
        wrapper_single_multicall.add_call(wrapper.wrapper(), true);
        wrapper_single_multicall.add_call(wrapper.conversion_selector(), true);
        wrapper_single_multicall.add_call(wrapper.base_unit(), true);
        let mut wrapper_double_multicall = self.multicall.clone();
        wrapper_double_multicall.clear_calls();
        wrapper_double_multicall.add_call(double.price_feed_one(), true);
        wrapper_double_multicall.add_call(double.price_feed_two(), true);
        wrapper_double_multicall.add_call(double.base_unit(), true);
        wrapper_double_multicall.add_call(double.decimals(), true);
        let wrapper_single_multicall = wrapper_single_multicall
            .version(MulticallVersion::Multicall)
            .block(meta.block_number);
        let wrapper_double_multicall = wrapper_double_multicall
            .version(MulticallVersion::Multicall)
            .block(meta.block_number);

        let (single_price_feed_result, double_price_feed_result) = tokio::join!(
            wrapper_single_multicall.call(),
            wrapper_double_multicall.call(),
        );

        let price_feed_address = if let Ok(wrapper_price_feed) = single_price_feed_result {
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
            price_feed.wrapper = Some(PriceFeedType::Single(PriceRate {
                address: wrapper,
                conversion_selector,
                base_unit,
                main_price: U256::zero(),
                rate: U256::zero(),
                event_emitter: None,
            }));
            vec![main_price_feed]
        } else if let Ok(wrapper) = double_price_feed_result {
            let (price_feed_one, price_feed_two, base_unit_double, decimals): (
                Address,
                Address,
                U256,
                U256,
            ) = wrapper;
            price_feed.wrapper = Some(PriceFeedType::Double(PriceDouble {
                price_feed_one,
                price_feed_two,
                base_unit: base_unit_double,
                decimals,
                price_one: U256::from(0u8),
                price_two: U256::from(0u8),
            }));
            vec![price_feed_one, price_feed_two]
        } else {
            vec![data.price_feed]
        };
        let mut aggregator_multicall = self.multicall.clone();
        aggregator_multicall.clear_calls();
        for p in &price_feed_address {
            aggregator_multicall.add_call(
                AggregatorProxy::new(*p, Arc::clone(&self.client)).aggregator(),
                false,
            );
        }
        let response = aggregator_multicall
            .block(meta.block_number)
            .version(MulticallVersion::Multicall)
            .call_raw()
            .await;
        let event_emitters = if let Ok(event_emitter) = response {
            let mut event_emitters = Vec::new();
            for aggregator in event_emitter {
                event_emitters.push(
                    aggregator
                        .map_err(|_| eyre!("wrong value"))?
                        .into_address()
                        .unwrap(),
                );
            }
            event_emitters
        } else {
            price_feed_address
        };
        price_feed.event_emitters = event_emitters;
        for (index, event_emitter) in price_feed.event_emitters.iter().enumerate() {
            self.data.contracts_to_listen.insert(
                ContractKey {
                    address: data.market,
                    kind: ContractKeyKind::PriceFeed,
                    index,
                },
                *event_emitter,
            );
        }
        let market = self
            .data
            .markets
            .entry(data.market)
            .or_insert_with_key(|key| Market::new(*key));
        market.price_feed = Some(price_feed);
        if let Some(PriceFeedType::Single(wrapper)) = market
            .price_feed
            .as_mut()
            .and_then(|price_feed| price_feed.wrapper)
            .as_mut()
        {
            let lido = PriceFeedLido::new(wrapper.address, Arc::clone(&self.client));
            let oracle = lido.get_oracle().block(meta.block_number).call().await?;
            wrapper.event_emitter = Some(oracle);
            self.data.contracts_to_listen.insert(
                ContractKey {
                    address: wrapper.address,
                    kind: ContractKeyKind::PriceFeed,
                    index: 0,
                },
                oracle,
            );
        };

        Ok(())
    }

    async fn check_liquidations(&self, block_number: U64, user: Option<Address>) -> Result<()> {
        let block = self
            .client
            .provider()
            .get_block(block_number)
            .await?
            .unwrap();
        let last_gas_price = block
            .base_fee_per_gas
            .map(|x| x + U256::from(1_500_000_000u128));

        let to_timestamp = block.timestamp;

        if self.data.comparison_enabled {
            self.compare_accounts(block_number, to_timestamp).await?;
        }

        let mut liquidations: HashMap<Address, (Account, Repay)> = HashMap::new();
        if let Some(address) = &user {
            let account = &self.data.accounts[address];
            self.check_liquidations_on_account(
                address,
                account,
                to_timestamp,
                last_gas_price,
                &mut liquidations,
            );
        } else {
            for (address, account) in &self.data.accounts {
                self.check_liquidations_on_account(
                    address,
                    account,
                    to_timestamp,
                    last_gas_price,
                    &mut liquidations,
                );
            }
        }
        info!(
            "accounts to liquidate {} / {} on block {} {}",
            liquidations.len(),
            self.data.accounts.len(),
            block_number,
            if self.config.simulation {
                " [SIMULATION]"
            } else {
                ""
            }
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
                        gas_price: last_gas_price,
                        markets: self.data.markets.keys().cloned().collect(),
                        assets: self
                            .data
                            .markets
                            .iter()
                            .map(|(address, market)| (*address, market.asset))
                            .collect(),
                        price_feeds: self
                            .data
                            .markets
                            .iter()
                            .map(|(address, market)| {
                                (*address, market.price_feed.as_ref().unwrap().address)
                            })
                            .collect(),
                        price_decimals: self.data.price_decimals,
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
        gas_price: Option<U256>,
        liquidations: &mut HashMap<Address, (Account, Repay)>,
    ) {
        let hf = Self::compute_hf(
            &self.data.markets,
            account,
            to_timestamp,
            Some(&self.config),
        );
        if let Ok((hf, _, _, repay)) = hf {
            if repay.total_adjusted_debt != U256::zero() {
                info!("{:<20?}: {:>28}", address, hf);
            }

            if hf < math::WAD
                && repay.total_adjusted_debt != U256::zero()
                && !self.config.simulation
            {
                let repay_settings = Liquidation::<M, W, S>::get_liquidation_settings(
                    &repay,
                    &self.data.liquidation_incentive,
                    self.data.repay_offset,
                    &self.token_pairs,
                    &self.tokens,
                );

                let (profitable, profit, cost) = Liquidation::<M, W, S>::is_profitable(
                    &self.network,
                    &repay,
                    &repay_settings,
                    &self.data.liquidation_incentive,
                    NetworkStatus {
                        gas_price,
                        gas_used: None,
                        eth_price: self.data.markets[&self.data.market_weth_address].price,
                    },
                );
                if profitable {
                    liquidations.insert(*address, (account.clone(), repay));
                } else {
                    liquidation::gen_liq_breadcrumb(account, &repay, &repay_settings);
                    warn!(
                        "liquidation not profitable for {:#?} (profit: {:#?} cost: {:#?})",
                        address, profit, cost
                    );
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
        use ethers::types::{Block, TransactionReceipt};
        use futures::TryFutureExt;
        use std::io::Write;

        use crate::generate_abi::MarketAccount;

        let mut previous_multicall = self.multicall.clone();
        previous_multicall.clear_calls();
        previous_multicall.add_call(
            self.auditor
                .account_liquidity(data.borrower, Address::zero(), U256::zero()),
            true,
        );
        previous_multicall.add_call(self.auditor.liquidation_incentive(), true);
        previous_multicall.add_call(self.previewer.exactly(data.borrower), true);
        let previous_multicall = previous_multicall
            .version(MulticallVersion::Multicall)
            .block(meta.block_number - 1u64);

        let mut current_multicall = self.multicall.clone();
        current_multicall.clear_calls();
        current_multicall.add_call(
            self.auditor
                .account_liquidity(data.borrower, Address::zero(), U256::zero()),
            true,
        );
        current_multicall.add_call(self.auditor.liquidation_incentive(), true);
        let current_multicall = current_multicall
            .version(MulticallVersion::Multicall)
            .block(meta.block_number);

        let mut price_multicall = self.multicall.clone();
        price_multicall.clear_calls();
        for market in self.data.markets.keys() {
            price_multicall.add_call(self.auditor.asset_price(*market), true);
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
                .map_err(|_| ethers::prelude::MulticallError::IllegalRevert),
            self.client
                .provider()
                .get_transaction_receipt(meta.transaction_hash)
                .map_err(|_| ethers::prelude::MulticallError::IllegalRevert),
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
            error!("error getting data from contracts on current block");
            return Ok(());
        };
        let current_block_data = current_block_data.unwrap();
        let timestamp = current_block_data.timestamp;
        let receipt = receipt.unwrap();
        let gas_used = receipt.gas_used.unwrap();
        let gas_cost = gas_used
            * receipt
                .effective_gas_price
                .unwrap()
                .mul_wad_down(self.data.markets[&self.data.market_weth_address].price);

        let market_prices: HashMap<Address, U256> = self
            .data
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
        let (_, _, _, repay) = Self::compute_hf(
            &self.data.markets,
            &self.data.accounts[&data.borrower],
            timestamp,
            None,
        )?;

        let close_factor =
            Liquidation::<M, W, S>::calculate_close_factor(&repay, &previous_liquidation_incentive);
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
            gas_used,
            gas_cost
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

        while skip < self.data.accounts.len() {
            let mut multicall = self.multicall.clone();
            multicall.clear_calls();
            let mut inputs: Vec<Address> = Vec::new();
            for account in self.data.accounts.keys().skip(skip).take(batch) {
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
        for _ in 0..(self.data.accounts.len() - 1 + batch) / batch {
            multicall_pool.push(self.multicall.clone());
        }
        self.data
            .accounts
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

        if !self.data.accounts.is_empty() {
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
            for (i, (token, (address, _))) in responses.iter().zip(&self.data.accounts).enumerate()
            {
                let v = token.clone().unwrap().into_tokens();
                let (adjusted_collateral, adjusted_debt): (U256, U256) = (
                    v[0].clone().into_uint().unwrap(),
                    v[1].clone().into_uint().unwrap(),
                );
                if adjusted_debt > U256::zero() {
                    let previewer_hf = adjusted_collateral.div_wad_down(adjusted_debt);
                    if let Some(account) = self.data.accounts.get(address) {
                        let _ = Self::compute_hf(&self.data.markets, account, timestamp, None).map(
                            |(hf, collateral, debt, _)| {
                                let print = i == 0;
                                success &= compare!(
                                    "health_factor",
                                    &account,
                                    "",
                                    previewer_hf,
                                    hf,
                                    print,
                                );
                                success &= compare!(
                                    "collateral",
                                    &account,
                                    "",
                                    adjusted_collateral,
                                    collateral,
                                );
                                success &=
                                    compare!("debt", &account, "", adjusted_debt, debt, print);
                                hf
                            },
                        );
                    }
                }
            }
        }
        if !success {
            let _ = cacache::remove(ProtocolData::cache_path(self.chain_id), CACHE_PROTOCOL).await;
            let mut data = BTreeMap::new();
            data.insert("block".to_string(), Value::Number(block.as_u64().into()));
            data.insert(
                "timestamp".to_string(),
                Value::String(timestamp.to_string()),
            );
            add_breadcrumb(Breadcrumb {
                ty: "error".to_string(),
                category: Some("compare".to_string()),
                level: Level::Error,
                data,
                ..Default::default()
            });
            panic!("compare accounts error");
        }
        Ok(())
    }

    #[cfg(feature = "complete-compare")]
    async fn compare_accounts(&self, block: U64, timestamp: U256) -> Result<()> {
        let previewer_accounts = &self.multicall_previewer(U64::from(&block)).await;
        let accounts = &self.data.accounts;
        let mut success = true;
        let mut compare_markets = true;
        let mut accounts_with_borrows = Vec::<Address>::new();

        if previewer_accounts.is_empty() {
            return Ok(());
        };
        for (address, previewer_account) in previewer_accounts {
            info!("Comparing account {:#?}", address);
            let account = &accounts[address];
            success &= compare!(
                "length",
                address,
                "[]",
                previewer_account.len(),
                account.positions.len()
            );
            if compare_markets {
                let mut multicall = self.multicall.clone();
                multicall.clear_calls();
                for market in self.data.markets.values() {
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
                self.data
                    .markets
                    .values()
                    .filter(|market| market.price_feed.is_some() && !market.base_market)
                    .zip(&responses)
                    .for_each(|(market, response)| {
                        success &= compare!(
                            "price",
                            "",
                            format!(
                                "{:#?}/{:#?}",
                                market.contract(Arc::clone(&self.client)).address(),
                                market.price_feed
                            ),
                            response.clone().into_uint().unwrap(),
                            market.price
                        );
                    });

                let mut multicall = self.multicall.clone();
                multicall.clear_calls();
                const FIELDS: usize = 9;
                for market in self.data.markets.values() {
                    multicall.add_call(
                        market.contract(Arc::clone(&self.client)).floating_assets(),
                        false,
                    );
                    multicall.add_call(
                        market
                            .contract(Arc::clone(&self.client))
                            .last_accumulator_accrual(),
                        false,
                    );
                    multicall.add_call(
                        market
                            .contract(Arc::clone(&self.client))
                            .earnings_accumulator(),
                        false,
                    );
                    multicall.add_call(
                        market
                            .contract(Arc::clone(&self.client))
                            .earnings_accumulator_smooth_factor(),
                        false,
                    );
                    multicall.add_call(
                        market.contract(Arc::clone(&self.client)).floating_debt(),
                        false,
                    );
                    multicall.add_call(
                        market
                            .contract(Arc::clone(&self.client))
                            .total_floating_borrow_shares(),
                        false,
                    );
                    multicall.add_call(
                        market
                            .contract(Arc::clone(&self.client))
                            .last_floating_debt_update(),
                        false,
                    );
                    multicall.add_call(
                        market
                            .contract(Arc::clone(&self.client))
                            .floating_utilization(),
                        false,
                    );
                    multicall.add_call(
                        market.contract(Arc::clone(&self.client)).total_supply(),
                        false,
                    );
                }
                let responses = multicall
                    .version(MulticallVersion::Multicall)
                    .block(block)
                    .call_raw()
                    .await?;
                for (i, market) in self.data.markets.values().enumerate() {
                    let previewer_market =
                        &previewer_account[&market.contract(Arc::clone(&self.client)).address()];

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
                let account = &self.data.accounts[address].positions[&market_account.market];
                success &= compare!(
                    "floating_deposit_assets",
                    address,
                    market_account.market,
                    market_account.floating_deposit_assets,
                    account.floating_deposit_assets::<M, S>(
                        &self.data.markets[&market_account.market],
                        timestamp
                    )
                );
                success &= compare!(
                    "floating_borrow_assets",
                    address,
                    market_account.market,
                    market_account.floating_borrow_assets,
                    account.floating_borrow_assets::<M, S>(
                        &self.data.markets[&market_account.market],
                        timestamp
                    )
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
            self.data.liquidation_incentive.liquidator
        );

        success &= compare!(
            "liquidation_incentive.lenders",
            "",
            "",
            previewer_incentive.1,
            self.data.liquidation_incentive.lenders
        );

        let mut multicall = self.multicall.clone();
        multicall.clear_calls();
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
            info!("account            : {}", account);
            info!("adjusted collateral: {}", adjusted_collateral);
            info!("adjusted debt      : {}", adjusted_debt);
            if adjusted_debt > U256::zero() {
                let previewer_hf = adjusted_collateral.div_wad_down(adjusted_debt);
                info!("health factor      : {}", previewer_hf);
                if let Some(account) = self.data.accounts.get(&account) {
                    if let Ok((hf, collateral, debt, _)) =
                        Self::compute_hf(&self.data.markets, account, timestamp, None)
                    {
                        success &= compare!("health_factor", &account, "", previewer_hf, hf);
                        success &=
                            compare!("collateral", &account, "", adjusted_collateral, collateral);
                        success &= compare!("debt", &account, "", adjusted_debt, debt);
                    }
                }
            }
        }
        if !success {
            let _ = cacache::remove(ProtocolData::cache_path(self.chain_id), CACHE_PROTOCOL).await;
            panic!("compare accounts error");
        }
        Ok(())
    }

    pub fn parse_abi(abi_path: &str) -> (Address, Abi, u64) {
        let file = File::open(abi_path).unwrap();
        let reader = BufReader::new(file);
        let contract: Value = serde_json::from_reader(reader).unwrap();
        let Value::Object(deployment) = contract else {
            panic!("Invalid ABI");
        };
        let (Some(contract), Some(abi)) = (
            deployment.get("address"),
            deployment.get("abi")
        ) else {
            panic!("Invalid ABI");
        };
        let (contract, abi) = (contract.clone(), abi.clone());
        let receipt = deployment.get("receipt").unwrap_or(&Value::Null).clone();
        let (Value::String(contract), Value::Array(abi)) = (contract, abi) else {
            panic!("Invalid ABI");
        };
        let contract = Address::from_str(&contract).unwrap();
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

    fn pick_adjust_factor(market: &Market, config: Option<&Config>) -> U256 {
        if let Some(config) = config {
            if config.simulation {
                if let Some(adjust_factor) = config.adjust_factor.get(&market.symbol) {
                    return *adjust_factor;
                }
            }
        }
        market.adjust_factor
    }

    fn compute_hf(
        markets: &HashMap<Address, Market>,
        account: &Account,
        timestamp: U256,
        config: Option<&Config>,
    ) -> Result<(U256, U256, U256, Repay)> {
        let mut total_collateral: U256 = U256::zero();
        let mut adjusted_collateral: U256 = U256::zero();
        let mut total_debt: U256 = U256::zero();
        let mut adjusted_debt: U256 = U256::zero();
        let mut repay = Repay::default();
        for (market_address, position) in account.positions.iter() {
            let market = markets.get(market_address).unwrap();
            let adjust_factor = Self::pick_adjust_factor(market, config);
            if position.is_collateral {
                let current_collateral =
                    position.floating_deposit_assets::<M, S>(market, timestamp);
                let value = current_collateral
                    .mul_div_down(market.price, U256::exp10(market.decimals as usize));
                total_collateral += value;
                adjusted_collateral += value.mul_wad_down(adjust_factor);
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
            current_debt += position.floating_borrow_assets::<M, S>(market, timestamp);

            let value =
                current_debt.mul_div_up(market.price, U256::exp10(market.decimals as usize));
            total_debt += value;
            adjusted_debt += value.div_wad_up(adjust_factor);
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
        Ok((hf, adjusted_collateral, adjusted_debt, repay))
    }
}

fn protocol_error_breadcrumb(e: &Report) {
    let mut data = BTreeMap::new();

    data.insert(
        "error message".to_string(),
        Value::String(format!("{:?}", e)),
    );

    sentry::add_breadcrumb(Breadcrumb {
        ty: "error".to_string(),
        category: Some("Protocol generic error".to_string()),
        level: Level::Error,
        data,
        ..Default::default()
    });
}

fn sentry_breadcrumb(meta: &LogMeta, topics: Vec<H256>, log_data: Vec<u8>, event: &ExactlyEvents) {
    let mut data = BTreeMap::new();
    data.insert(
        "address".to_string(),
        Value::String(format!("{:X}", meta.address)),
    );
    data.insert(
        "block_number".to_string(),
        Value::Number(meta.block_number.as_u64().into()),
    );
    data.insert(
        "block_hash".to_string(),
        Value::String(format!("{:X}", meta.block_hash)),
    );
    data.insert(
        "transaction_hash".to_string(),
        Value::String(format!("{:X}", meta.transaction_hash)),
    );
    data.insert(
        "transaction_index".to_string(),
        Value::Number(meta.transaction_index.as_u64().into()),
    );
    data.insert(
        "log_index".to_string(),
        Value::Number(meta.log_index.as_u64().into()),
    );
    data.insert(
        "topics".to_string(),
        Value::Array(
            topics
                .iter()
                .map(|t| Value::String(format!("{:X}", t)))
                .collect(),
        ),
    );
    data.insert(
        "data".to_string(),
        Value::String(format!("{:02X?}", log_data)),
    );
    let event_name = format!("{:?}", event);
    add_breadcrumb(Breadcrumb {
        ty: "info".to_string(),
        category: Some(event_name[..event_name.find('(').unwrap_or(event_name.len())].to_string()),
        level: Level::Info,
        data,
        ..Default::default()
    });
}

pub mod hashmap_as_vector {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::iter::FromIterator;

    pub fn serialize<'a, T, K, V, S>(target: T, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: IntoIterator<Item = (&'a K, &'a V)>,
        K: Serialize + 'a,
        V: Serialize + 'a,
    {
        let container: Vec<_> = target.into_iter().collect();
        serde::Serialize::serialize(&container, ser)
    }

    pub fn deserialize<'de, T, K, V, D>(des: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: FromIterator<(K, V)>,
        K: Deserialize<'de>,
        V: Deserialize<'de>,
    {
        let container: Vec<_> = serde::Deserialize::deserialize(des)?;
        Ok(T::from_iter(container.into_iter()))
    }
}
