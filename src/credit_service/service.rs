use crate::bindings::ExactlyEvents;
use crate::config::Config;
use crate::credit_service::{Account, AggregatorProxy, Auditor, FixedLender, Market};
use ethers::abi::Tokenizable;
use ethers::prelude::{
    abi::{Abi, RawLog},
    types::Filter,
    Address, EthLogDecode, LogMeta, Middleware, Multicall, Signer, SignerMiddleware, U256, U64,
};
use ethers::prelude::{Log, PubsubClient};
use eyre::Result;
use serde_json::Value;
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
    InterestRateModel,
    Oracle,
    Auditor,
}

enum LogIterating {
    NextLog,
    UpdateFilters,
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
pub struct CreditService<M, S> {
    client: Arc<SignerMiddleware<M, S>>,
    last_sync: (U64, i128, i128),
    previewer: Previewer<SignerMiddleware<M, S>>,
    auditor: Auditor<SignerMiddleware<M, S>>,
    oracle: ExactlyOracle<SignerMiddleware<M, S>>,
    liquidator: Liquidator<SignerMiddleware<M, S>>,
    markets: HashMap<Address, FixedLender<M, S>>,
    sp_fee_rate: U256,
    borrowers: HashMap<Address, Account>,
    contracts_to_listen: HashMap<ContractKey, Address>,
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
        let mut markets = HashMap::<Address, FixedLender<M, S>>::new();
        for market in auditor_markets {
            println!("Adding market {:?}", market);
            markets
                .entry(market)
                .or_insert_with_key(|key| FixedLender::new(*key, &client));
        }

        let mut contracts_to_listen = HashMap::new();
        markets.keys().for_each(|address| {
            contracts_to_listen.insert(
                ContractKey {
                    address: *address,
                    kind: ContractKeyKind::Market,
                },
                *address,
            );
        });

        contracts_to_listen.insert(
            ContractKey {
                address: (*auditor).address(),
                kind: ContractKeyKind::Auditor,
            },
            (*auditor).address(),
        );

        let interest_rate_model_address =
            Address::from_str("0xeC00E4A3f1c170E57f0261632c139f8330BAfbA3")?;
        contracts_to_listen.insert(
            ContractKey {
                address: interest_rate_model_address,
                kind: ContractKeyKind::InterestRateModel,
            },
            interest_rate_model_address,
        );

        Ok(CreditService {
            client: Arc::clone(&client),
            last_sync: (U64::from(deployed_block), -1, -1),
            auditor,
            previewer,
            oracle,
            liquidator,
            markets,
            sp_fee_rate: U256::zero(),
            borrowers: HashMap::new(),
            contracts_to_listen,
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
            market.contract = Market::new(address, Arc::clone(&client));
        }
    }

    pub async fn launch(self) -> Result<Self, Self>
    where
        <M as Middleware>::Provider: PubsubClient,
    {
        let service = Arc::new(Mutex::new(self));
        let (debounce_tx, mut debounce_rx) = mpsc::channel(10);
        let me = Arc::clone(&service);
        let a = tokio::spawn(async move {
            let mut block_number = None;
            loop {
                match time::timeout(Duration::from_millis(2_000), debounce_rx.recv()).await {
                    Ok(Some(block)) => {
                        block_number = block;
                        println!("just received log");
                    }
                    Ok(None) => {
                        println!("end of stream");
                    }
                    Err(_) => {
                        println!("{:?}ms since network activity", 2_000);
                        if let Some(block) = block_number {
                            let _ = me.lock().await.check_liquidations(block).await;
                        }
                    }
                }
            }
        });
        println!(
            "=================test===================== {:#?}",
            service.lock().await.last_sync
        );
        'filter: loop {
            let client;
            let stream_error;
            {
                let service_unlocked = service.lock().await;
                println!("update filter: {:#?}", service_unlocked.contracts_to_listen);
                let filter = Filter::new()
                    .from_block(service_unlocked.last_sync.0)
                    .address(
                        service_unlocked
                            .contracts_to_listen
                            .values()
                            .cloned()
                            .collect::<Vec<Address>>(),
                    );

                client = Arc::clone(&service_unlocked.client);
                stream_error = client.subscribe_logs(&filter).await;
            }
            match stream_error {
                Ok(mut stream) => {
                    let mut last_block = U64::zero();
                    while let Some(log) = stream.next().await {
                        if let Some(block_number) = log.block_number {
                            if block_number != last_block {
                                if last_block > U64::zero() {
                                    let to_timestamp = service
                                        .lock()
                                        .await
                                        .client
                                        .provider()
                                        .get_block(last_block)
                                        .await
                                        .unwrap()
                                        .unwrap()
                                        .timestamp;
                                    service
                                        .lock()
                                        .await
                                        .compare_accounts(U64::from(last_block), to_timestamp)
                                        .await
                                        .unwrap();
                                }
                                last_block = block_number;
                            }
                        }
                        let mut me = service.lock().await;
                        let status = me.handle_log(log, &debounce_tx).await;
                        match status {
                            Ok(result) => {
                                if let LogIterating::UpdateFilters = result {
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
                _ => {
                    println!("error to subscribe");
                    break;
                }
            };
        }
        a.abort();
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
    async fn handle_log(&mut self, log: Log, sender: &Sender<Option<U64>>) -> Result<LogIterating> {
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
        if (
            meta.block_number,
            meta.transaction_index.as_u64() as i128,
            meta.log_index.as_u128() as i128,
        ) <= self.last_sync
        {
            return Ok(LogIterating::NextLog);
        }
        println!("{:?}, {:?}", event, meta);
        match event {
            ExactlyEvents::MaxFuturePoolsSetFilter(data) => {
                self.markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| FixedLender::new(*key, &self.client))
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
                sender.send(log.block_number).await?;
                return Ok(LogIterating::UpdateFilters);
            }

            ExactlyEvents::EarningsAccumulatorSmoothFactorSetFilter(data) => {
                self.markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| FixedLender::new(*key, &self.client))
                    .accumulated_earnings_smooth_factor = data.earnings_accumulator_smooth_factor;
            }

            ExactlyEvents::MarketListedFilter(data) => {
                let mut market = self
                    .markets
                    .entry(data.market)
                    .or_insert_with_key(|key| FixedLender::new(*key, &self.client));
                market.decimals = data.decimals;
                market.smart_pool_fee_rate = self.sp_fee_rate;
                market.listed = true;
                market.approve_asset(&self.client).await?;
                self.update_prices(meta.block_number).await?;
            }

            ExactlyEvents::TransferFilter(data) => {
                if data.from != data.to
                    && (data.from == Address::zero() || data.to == Address::zero())
                {
                    // let market = self
                    //     .markets
                    //     .entry(meta.address)
                    //     .or_insert_with_key(|key| FixedLender::new(*key));
                }
            }
            ExactlyEvents::DepositFilter(data) => {
                println!("number of borrowers - 0: {:#?}", self.borrowers.len());
                self.borrowers
                    .entry(data.owner)
                    .or_insert_with(|| Account::new(data.owner, &self.markets))
                    .deposit(&data, &meta.address);
                println!("number of borrowers - 1: {:#?}", self.borrowers.len());
                if data.owner
                    == Address::from_str("0x9ca61a949242ea4cd131f11f7ccd9b63148654ea").unwrap()
                {
                    if let Some(borrower) = self.borrowers.get(&data.owner) {
                        println!(">>>> borrower positions: {:#?}", borrower.positions);
                    } else {
                        println!(">>>> no such borrower");
                    }
                }
            }
            ExactlyEvents::WithdrawFilter(data) => {
                self.borrowers
                    .entry(data.owner)
                    .or_insert_with(|| Account::new(data.owner, &self.markets))
                    .withdraw(&data, &meta.address);
                if data.owner
                    == Address::from_str("0x9ca61a949242ea4cd131f11f7ccd9b63148654ea").unwrap()
                {
                    if let Some(borrower) = self.borrowers.get(&data.owner) {
                        println!(">>>> borrower positions: {:#?}", borrower.positions);
                    } else {
                        println!(">>>> no such borrower");
                    }
                }
            }
            ExactlyEvents::DepositAtMaturityFilter(data) => {
                self.borrowers
                    .entry(data.owner)
                    .or_insert_with(|| Account::new(data.owner, &self.markets))
                    .deposit_at_maturity(&data, &meta.address);
            }
            ExactlyEvents::WithdrawAtMaturityFilter(data) => {
                self.borrowers
                    .entry(data.owner)
                    .or_insert_with(|| Account::new(data.owner, &self.markets))
                    .withdraw_at_maturity(data, &meta.address);
            }
            ExactlyEvents::BorrowAtMaturityFilter(data) => {
                self.borrowers
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.markets))
                    .borrow_at_maturity(&data, &meta.address);
            }
            ExactlyEvents::RepayAtMaturityFilter(data) => {
                self.borrowers
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.markets))
                    .repay_at_maturity(&data, &meta.address);
            }
            ExactlyEvents::LiquidateFilter(data) => {
                self.borrowers
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.markets))
                    .liquidate_borrow(data, &meta.address);
            }
            ExactlyEvents::SeizeFilter(data) => {
                self.borrowers
                    .entry(data.borrower)
                    .or_insert_with(|| Account::new(data.borrower, &self.markets))
                    .asset_seized(data, &meta.address);
            }

            ExactlyEvents::AdjustFactorSetFilter(data) => {
                self.markets
                    .entry(data.market)
                    .or_insert_with_key(|key| FixedLender::new(*key, &self.client))
                    .adjust_factor = data.adjust_factor;
            }

            ExactlyEvents::PenaltyRateSetFilter(data) => {
                self.markets
                    .entry(meta.address)
                    .or_insert_with_key(|key| FixedLender::new(*key, &self.client))
                    .penalty_rate = data.penalty_rate;
            }

            ExactlyEvents::MarketEnteredFilter(data) => {
                self.borrowers
                    .entry(data.account)
                    .or_insert_with(|| Account::new(data.account, &self.markets))
                    .set_collateral(&data.market);
            }

            ExactlyEvents::MarketExitedFilter(data) => {
                self.borrowers
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
                    .or_insert_with_key(|key| FixedLender::new(*key, &self.client))
                    .price_feed = aggregator;
                self.update_prices(meta.block_number).await?;
                self.last_sync = (
                    meta.block_number,
                    meta.transaction_index.as_u64() as i128,
                    meta.log_index.as_u128() as i128,
                );
                sender.send(log.block_number).await?;
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
                    .or_insert_with_key(|key| FixedLender::new(*key, &self.client))
                    .oracle_price = data.current.into_raw() * U256::exp10(10);
            }

            ExactlyEvents::MarketUpdateFilter(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| FixedLender::new(*address, &self.client));
                market.total_shares = data.floating_deposit_shares;
                market.smart_pool_assets = data.floating_assets;
                market.smart_pool_earnings_accumulator = data.earnings_accumulator;
            }

            ExactlyEvents::AccumulatorAccrualFilter(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| FixedLender::new(*address, &self.client));
                market.last_accumulated_earnings_accrual = data.timestamp;
            }

            ExactlyEvents::FixedEarningsUpdateFilter(data) => {
                let market = self
                    .markets
                    .entry(meta.address)
                    .or_insert_with_key(|address| FixedLender::new(*address, &self.client));

                let pool = market.fixed_pools.entry(data.maturity).or_default();
                pool.last_accrual = data.timestamp;
                pool.unassigned_earnings = data.unassigned_earnings;
            }

            _ => {
                println!("Event not handled - {:?}", event);
            }
        }

        sender.send(log.block_number).await?;
        Ok(LogIterating::NextLog)
    }

    async fn check_liquidations(&mut self, block_number: U64) -> Result<()> {
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

        self.compare_accounts(block_number, to_timestamp).await?;

        let mut liquidations: HashMap<Address, Account> = HashMap::new();
        for (address, borrower) in self.borrowers.iter_mut() {
            let hf = Self::compute_hf(&mut self.markets, borrower, to_timestamp);
            if let Ok(hf) = hf {
                if hf < U256::exp10(18) && borrower.debt() != U256::zero() {
                    liquidations.insert(address.clone(), borrower.clone());
                }
            }
        }
        self.liquidate(&liquidations).await?;
        Ok(())
    }

    async fn liquidate(&self, liquidations: &HashMap<Address, Account>) -> Result<()> {
        for (_, borrower) in liquidations {
            println!("Liquidating borrower {:?}", borrower);
            if let Some(address) = &borrower.fixed_lender_to_liquidate() {
                println!("Liquidating on fixed lender {:?}", address);

                // let contract =
                //     Market::<SignerMiddleware<M, S>>::new(*address, Arc::clone(&&self.client));

                let func = self
                    .liquidator
                    .liquidate(
                        *address,
                        borrower.seizable_collateral().unwrap(),
                        borrower.address(),
                        U256::MAX,
                        Address::zero(),
                        0,
                        0,
                    )
                    .gas(6_666_666);

                let tx = func.send().await;
                println!("tx: {:?}", &tx);
                let tx = tx?;
                let receipt = tx.await?;
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
        let batch = 1_000;
        let mut positions = HashMap::<Address, HashMap<Address, MarketAccount>>::new();

        while skip < self.borrowers.len() {
            let mut multicall =
                Multicall::<SignerMiddleware<M, S>>::new(Arc::clone(&self.client), None)
                    .await
                    .unwrap();
            let mut inputs: Vec<Address> = Vec::new();
            for borrower in self.borrowers.keys().skip(skip).take(batch) {
                inputs.push(*borrower);

                multicall.add_call(self.previewer.exactly(*borrower));
            }
            let responses = multicall.block(block).call_raw().await.unwrap();

            let mut accounts_updated = inputs.iter();
            for response in responses {
                let payload: Vec<MarketAccount> = Vec::from_token(response).unwrap();
                let borrower = accounts_updated
                    .next()
                    .expect("Number of self.borrowers and responses doesn't match");

                let mut markets = HashMap::new();
                for market_account in payload {
                    markets.insert(market_account.market, market_account);
                }
                positions.insert(*borrower, markets);
            }

            skip += batch;
        }
        positions
    }

    async fn compare_accounts(&self, block: U64, timestamp: U256) -> Result<()> {
        let previewer_accounts = &self.multicall_previewer(U64::from(&block)).await;
        let accounts = &self.borrowers;
        let mut success = true;
        let mut compare_markets = true;
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
                for market in self.markets.values() {
                    multicall.add_call(market.contract.floating_assets());
                    multicall.add_call(market.contract.last_accumulator_accrual());
                    multicall.add_call(market.contract.earnings_accumulator());
                    multicall.add_call(market.contract.earnings_accumulator_smooth_factor());
                }
                let responses = multicall.block(block).call_raw().await?;
                for (i, market) in self.markets.values().enumerate() {
                    let previewer_market = &previewer_account[&market.contract.address()];

                    success &= compare!(
                        "floating_assets",
                        "",
                        previewer_market.market,
                        responses[i * 4].clone().into_uint().unwrap(),
                        market.smart_pool_assets
                    );
                    success &= compare!(
                        "last_accumulator_accrual",
                        "",
                        previewer_market.market,
                        responses[i * 4 + 1].clone().into_uint().unwrap(),
                        market.last_accumulated_earnings_accrual
                    );
                    success &= compare!(
                        "earnings_accumulator",
                        "",
                        previewer_market.market,
                        responses[i * 4 + 2].clone().into_uint().unwrap(),
                        market.smart_pool_earnings_accumulator
                    );
                    success &= compare!(
                        "earnings_accumulator_smooth_factor",
                        "",
                        previewer_market.market,
                        responses[i * 4 + 3].clone().into_uint().unwrap(),
                        U256::from(market.accumulated_earnings_smooth_factor)
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
                }
            }
            compare_markets = false;
            // for previewer_market in previewer_account {
            //     let market = &self.markets[&previewer_market.market];
            // }
        }
        if !success {
            panic!("compare accounts error");
        }
        Ok(())
    }

    //     async fn _compare_positions(&self, block: U64, timestamp: U256) -> Result<()> {
    //         let previewer_borrowers = &self.multicall_previewer(U64::from(&block)).await;
    //         let event_borrowers = &self.borrowers;
    //         println!("Comparing positions");
    //         println!("-------------------");
    //         for (account, previewer_borrower) in previewer_borrowers {
    //             let event_borrower = &event_borrowers[account];
    //             println!("Account: {:?}", account);
    //             println!("---");
    //             if previewer_borrower.len() != event_borrower.data().len() {
    //                 println!(
    //                     "Number of fixed lenders doesn't match. Previewer: {:?}, Event: {:?}",
    //                     previewer_borrower.len(),
    //                     event_borrower.data().len()
    //                 );
    //             } else if previewer_borrower.len() == 0 {
    //                 println!("No markets.");
    //             } else {
    //                 for previewer_account in previewer_borrower.iter() {
    //                     let market_address = previewer_account.market;

    //                     if let Some(event_account) = event_borrower.data().get(&market_address) {
    //                         println!("Market: {:?}", market_address);
    //                         let mut _fixed_lender_correct = true;

    //                         let contract = Market::new(market_address, self.client.clone());
    //                         let total_assets =
    //                             contract.total_assets().block(block).call().await.unwrap();

    //                         let total_assets_event =
    //                             self.markets[&market_address].total_assets(timestamp);

    //                         const INTERVAL: u32 = 4 * 7 * 86_400;

    //                         let latest = ((timestamp - (timestamp % INTERVAL)) / INTERVAL).as_u32();
    //                         let max_future_pools = self.markets[&market_address].max_future_pools;
    //                         println!(
    //                             "latest: {}, max_future_pools: {}, INTERVAL: {}, timestamp: {}",
    //                             latest, max_future_pools, INTERVAL, timestamp
    //                         );

    //                         if false {
    //                             let event_account_market = &self.markets[&market_address];
    //                             let event_account_fixed_pools = &event_account_market.fixed_pools;
    //                             for i in latest..=latest + max_future_pools as u32 {
    //                                 let maturity = i * INTERVAL;
    //                                 let maturity_result = contract
    //                                     .fixed_pools(U256::from(maturity))
    //                                     .block(block)
    //                                     .call()
    //                                     .await
    //                                     .unwrap();

    //                                 if event_account_fixed_pools.contains_key(&maturity) {
    //                                     let maturity_local =
    //                                         event_account_fixed_pools.get(&maturity).unwrap();
    //                                     println!("Maturity: {:?}", maturity);
    //                                     println!(
    //                                         "Maturity: {:?} {:?}",
    //                                         maturity_result.0, maturity_local.borrowed
    //                                     );
    //                                     println!(
    //                                         "Maturity: {:?} {:?}",
    //                                         maturity_result.1, maturity_local.supplied
    //                                     );
    //                                     println!(
    //                                         "Maturity: {:?} {:?}",
    //                                         maturity_result.2, maturity_local.unassigned_earnings
    //                                     );
    //                                     println!(
    //                                         "Maturity: {:?} {:?}",
    //                                         maturity_result.3, maturity_local.last_accrual
    //                                     );
    //                                     if maturity_result.0 != maturity_local.borrowed
    //                                         || maturity_result.1 != maturity_local.supplied
    //                                         || maturity_result.2 != maturity_local.unassigned_earnings
    //                                         || maturity_result.3 != maturity_local.last_accrual
    //                                     {
    //                                         // panic!("Maturity");
    //                                     }
    //                                 } else {
    //                                     println!("Maturity not in events");
    //                                 }
    //                             }
    //                         }

    //                         let market = self.markets.get(&market_address).unwrap();

    //                         if &previewer_account.floating_deposit_assets
    //                             != &event_account.floating_deposit_assets(&market, timestamp)
    //                         {
    //                             _fixed_lender_correct = false;
    //                             println!("  smart_pool_assets:");
    //                             println!(
    //                                 "    Previewer: {:?}",
    //                                 &previewer_account.floating_deposit_assets
    //                             );
    //                             println!(
    //                                 "    Event    : {:?}",
    //                                 &event_account.floating_deposit_assets(&market, timestamp)
    //                             );
    //                             println!("\nMarket: {:?}", event_account);
    //                             println!("total_assets: {}\n", market.total_assets(timestamp));

    //                             println!(
    //                                 "
    // total_assets = {:?}",
    //                                 total_assets
    //                             );

    //                             // panic!("Debugging");
    //                         }

    //                         if total_assets != total_assets_event {
    //                             println!("  event: {}", total_assets_event);
    //                             println!("  previewer: {}", total_assets);
    //                             panic!("Debugging");
    //                         }

    //                         if previewer_account.floating_deposit_shares
    //                             != event_account.smart_pool_shares
    //                         {
    //                             _fixed_lender_correct = false;
    //                             println!("  smart_pool_shares:");
    //                             println!(
    //                                 "    Previewer: {:?}",
    //                                 &previewer_account.floating_deposit_shares
    //                             );
    //                             println!("    Event    : {:?}", &event_account.smart_pool_shares);
    //                             panic!("Debugging");
    //                         }
    //                         if previewer_account.oracle_price != market.oracle_price {
    //                             _fixed_lender_correct = false;
    //                             println!("  oracle_price:");
    //                             println!("    Previewer: {:?}", &previewer_account.oracle_price);
    //                             println!("    Event    : {:?}", &market.oracle_price);
    //                             panic!("Debugging");
    //                         }
    //                         if U256::from(previewer_account.penalty_rate) != market.penalty_rate {
    //                             _fixed_lender_correct = false;
    //                             println!("  penalty_rate:");
    //                             println!("    Previewer: {:?}", &previewer_account.penalty_rate);
    //                             println!("    Event    : {:?}", &market.penalty_rate);
    //                         }
    //                         if U256::from(previewer_account.adjust_factor) != market.adjust_factor {
    //                             _fixed_lender_correct = false;
    //                             println!("  adjust_factor:");
    //                             println!("    Previewer: {:?}", &previewer_account.adjust_factor);
    //                             println!("    Event    : {:?}", &market.adjust_factor);
    //                             panic!("Debugging");
    //                         }
    //                         if previewer_account.decimals != market.decimals {
    //                             _fixed_lender_correct = false;
    //                             println!("  decimals:");
    //                             println!("    Previewer: {:?}", previewer_account.decimals);
    //                             println!("    Event    : {:?}", &market.decimals);
    //                             panic!("Debugging");
    //                         }
    //                         if previewer_account.is_collateral != event_account.is_collateral {
    //                             _fixed_lender_correct = false;
    //                             println!("  is_collateral:");
    //                             println!("    Previewer: {:?}", &previewer_account.is_collateral);
    //                             println!("    Event    : {:?}", &event_account.is_collateral);
    //                             panic!("Debugging");
    //                         }

    //                         let mut _supplies_correct = true;
    //                         for position in &previewer_account.fixed_deposit_positions {
    //                             let total_supply = event_account.maturity_supply_positions
    //                                 [&position.maturity.as_u32()];
    //                             let total = position.position.principal + position.position.fee;

    //                             if total != total_supply {
    //                                 _supplies_correct = false;
    //                                 println!("  supplies:");
    //                                 println!("    Previewer: {:?}", &total);
    //                                 println!("    Event    : {:?}", &total_supply);
    //                                 panic!("Debugging");
    //                             }
    //                         }
    //                         // for (maturity, total) in event_account.maturity_supply_positions {
    //                         //     if previewer_account.maturity_supply_positions[&maturity] == None {
    //                         //         supplies_correct = false;
    //                         //         println!("    Maturity {:?} not found on previewer", maturity);
    //                         //         panic!("Debugging");
    //                         //     }
    //                         // }

    //                         let mut _borrows_correct = true;
    //                         for position in &previewer_account.fixed_borrow_positions {
    //                             let borrowed_total = &event_account.maturity_borrow_positions
    //                                 [&position.maturity.as_u32()];
    //                             let total = position.position.principal + position.position.fee;
    //                             if total != *borrowed_total {
    //                                 _borrows_correct = false;
    //                                 println!("  borrows:");
    //                                 println!("    Previewer: {:?}", &total);
    //                                 println!("    Event    : {:?}", &borrowed_total);
    //                                 panic!("Debugging");
    //                             }
    //                         }
    //                         // for (maturity, total) in event_account.maturity_borrow_positions() {
    //                         //     if previewer_account.borrow_at_maturity(&maturity) == None {
    //                         //         borrows_correct = false;
    //                         //         println!("    Maturity {:?} not found on previewer", maturity);
    //                         //         panic!("Debugging");
    //                         //     }
    //                         // }
    //                     } else {
    //                         println!(
    //                             "Fixed lender {:?} doesn't found in data generated by events.",
    //                             &previewer_account.market
    //                         );
    //                     }
    //                 }
    //             }
    //             println!("-------------------\n");
    //         }
    //         Ok(())
    //     }

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

    pub fn compute_hf(
        markets: &mut HashMap<Address, FixedLender<M, S>>,
        account: &mut Account,
        timestamp: U256,
    ) -> Result<U256> {
        let mut collateral: U256 = U256::zero();
        let mut debt: U256 = U256::zero();
        let mut seizable_collateral: (U256, Option<Address>) = (U256::zero(), None);
        let mut fixed_lender_to_liquidate: (U256, Option<Address>) = (U256::zero(), None);
        for (market_address, position) in account.positions.iter() {
            let market = markets.get_mut(market_address).unwrap();
            if position.is_collateral {
                let current_collateral = position.floating_deposit_assets(market, timestamp)
                    * market.oracle_price
                    * U256::from(market.adjust_factor)
                    / U256::exp10(market.decimals as usize)
                    / U256::exp10(18);
                if current_collateral > seizable_collateral.0 {
                    seizable_collateral = (current_collateral, Some(*market_address));
                }
                collateral += current_collateral;
            }
            let mut current_debt = U256::zero();
            for (maturity, borrowed) in position.maturity_borrow_positions.iter() {
                current_debt += *borrowed;
                if U256::from(*maturity) < timestamp {
                    current_debt +=
                        (timestamp - U256::from(*maturity)) * U256::from(market.penalty_rate)
                }
            }
            debt += current_debt * market.oracle_price * U256::exp10(18)
                / U256::from(market.adjust_factor)
                / U256::exp10(market.decimals as usize);
            if current_debt > fixed_lender_to_liquidate.0 {
                fixed_lender_to_liquidate = (current_debt, Some(*market_address));
            }
        }
        account.collateral = Some(collateral);
        account.seizable_collateral = seizable_collateral.1;
        account.fixed_lender_to_liquidate = fixed_lender_to_liquidate.1;
        account.debt = Some(debt);
        let hf = if debt == U256::zero() {
            collateral
        } else {
            U256::exp10(18) * collateral / debt
        };
        println!("==============");
        println!("Borrower               {:?}", account.address);
        println!("Total Collateral       {:?}", collateral);
        println!("Total Debt             {:?}", debt);
        println!("Seizable Collateral    {:?}", seizable_collateral.1);
        println!("Seizable Collateral  $ {:?}", seizable_collateral.0);
        println!("Debt on Fixed Lender   {:?}", fixed_lender_to_liquidate.1);
        println!("Debt on Fixed Lender $ {:?}", fixed_lender_to_liquidate.0);
        println!("Health factor {:?}\n", hf);
        Ok(hf)
    }
}
