use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::str::FromStr;
use std::sync::Arc;
use std::{thread, time};

use anyhow::{Error, Result};
use ethers::abi::{Abi, RawLog};
use ethers::core::types::Filter;
use ethers::core::types::{Address, U256, U64};
use ethers::prelude::k256::ecdsa::SigningKey;
use ethers::prelude::{
    ContractError, EthLogDecode, Http, LogMeta, Middleware, Multicall, Provider, SignerMiddleware,
    StreamExt, Wallet,
};
use serde_json::Value;

use crate::bindings::ExactlyEvents;
use crate::config::Config;
use crate::credit_service::{Account, Auditor, FixedLender, Market};

use super::{ExactlyOracle, MarketAccount, Previewer};

#[derive(Eq, PartialEq, Hash, Clone, Copy)]
enum ContractKeyKind {
    Market,
    PriceFeed,
    InterestRateModel,
    Oracle,
    Auditor,
}

#[derive(Eq, PartialEq, Hash, Clone, Copy)]
struct ContractKey {
    address: Address,
    kind: ContractKeyKind,
}

pub struct CreditService {
    previewer: Previewer<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>,
    client: Arc<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>,
    last_sync: (U64, i128, i128),
    auditor: Auditor<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>,
    provider: Provider<Http>,
    oracle: ExactlyOracle<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>,
    markets: HashMap<Address, FixedLender>,
    sp_fee_rate: U256,
    borrowers: HashMap<Address, Account>,
    contracts_to_listen: HashMap<ContractKey, Address>,
}

impl CreditService {
    pub fn new(
        client: Arc<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>,
        provider: Provider<Http>,
        config: Config,
    ) -> CreditService {
        let (auditor_address, _, block_number) = CreditService::parse_abi(&format!(
            "lib/protocol/deployments/{}/Auditor.json",
            config.chain_id_name
        ));
        let auditor = Auditor::new(auditor_address, Arc::clone(&client));

        let (previewer_address, _, _) = CreditService::parse_abi(&format!(
            "lib/protocol/deployments/{}/Previewer.json",
            config.chain_id_name
        ));
        let previewer = Previewer::new(previewer_address, Arc::clone(&client));

        CreditService {
            previewer,
            client: Arc::clone(&client),
            last_sync: (U64::from(block_number - 1), -1, -1),
            auditor,
            provider,
            oracle: ExactlyOracle::new(Address::zero(), Arc::clone(&client)),
            markets: HashMap::<Address, FixedLender>::new(),
            sp_fee_rate: U256::zero(),
            borrowers: HashMap::new(),
            contracts_to_listen: HashMap::new(),
        }
    }

    pub async fn launch(&mut self) -> Result<(), Error> {
        let markets = self.auditor.get_all_markets().call().await?;
        for market in markets {
            println!("Adding market {:?}", market);
            self.markets
                .entry(market)
                .or_insert_with_key(|key| FixedLender::new(*key));
        }

        self.markets.keys().for_each(|address| {
            self.contracts_to_listen
                .entry(ContractKey {
                    address: *address,
                    kind: ContractKeyKind::Market,
                })
                .or_insert_with_key(|key| key.address);
        });

        self.contracts_to_listen
            .entry(ContractKey {
                address: (*self.auditor).address(),
                kind: ContractKeyKind::Auditor,
            })
            .or_insert_with_key(|key| key.address);

        let interest_rate_model_address =
            Address::from_str("0xeC00E4A3f1c170E57f0261632c139f8330BAfbA3")?;
        self.contracts_to_listen
            .entry(ContractKey {
                address: interest_rate_model_address,
                kind: ContractKeyKind::InterestRateModel,
            })
            .or_insert_with_key(|key| key.address);

        self.update().await?;

        let watcher = self.client.clone();
        let mut on_block = watcher
            .watch_blocks()
            .await
            .map_err(
                ContractError::MiddlewareError::<
                    SignerMiddleware<Provider<Http>, Wallet<SigningKey>>,
                >,
            )
            .unwrap()
            .stream();
        while on_block.next().await.is_some() {
            match self.update().await {
                Err(e) => {
                    println!("{}", &e);
                }
                _ => {}
            }

            println!("zzzzz...");
            let delay = time::Duration::from_secs(20);
            thread::sleep(delay);
        }
        Ok(())
    }

    async fn update_prices(&mut self, block: U64) -> Result<()> {
        println!("\nUpdating prices for block {}:\n", block);
        let mut multicall = Multicall::<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>::new(
            Arc::clone(&self.client),
            None,
        )
        .await?;
        let mut update = false;
        for (market, _) in self.markets.iter().filter(|(_, v)| v.listed) {
            update = true;
            multicall.add_call(
                self.oracle
                    .method::<Address, U256>("getAssetPrice", *market)?,
            );
        }
        if !update {
            return Ok(());
        }
        println!("call multicall for updating prices");
        let result = multicall.block(block).call_raw().await?;
        for (i, market) in self.markets.values_mut().filter(|m| m.listed).enumerate() {
            market.oracle_price = result[i].clone().into_uint().unwrap();
        }

        Ok(())
    }

    async fn handle_events(
        &mut self,
        logs: Vec<(ExactlyEvents, LogMeta)>,
        from: (U64, i128, i128),
    ) -> Result<(U64, i128, i128), Error> {
        let mut block = from.0;
        let mut block_timestamp = U256::zero();
        for (event, meta) in logs {
            println!(
                "---->     Contract {:?} - {}",
                meta.address, meta.block_number
            );
            if (
                meta.block_number,
                meta.transaction_index.as_u64() as i128,
                meta.log_index.as_u128() as i128,
            ) <= from
            {
                continue;
            }
            if meta.block_number > block {
                println!("----> Block: {}", meta.block_number);
                // When enabled, creates comparison block by block
                if false {
                    if self.borrowers.len() > 0 {
                        // let previewer_borrowers = self.multicall_previewer(block).await;
                        if (*self.oracle).address() != Address::zero() {
                            self.update_prices(meta.block_number).await?;
                        }
                        println!("Partial compare");
                        self.compare_positions(block, block_timestamp).await?;
                    }
                }
                block = meta.block_number;
                block_timestamp = self
                    .provider
                    .get_block(meta.block_number)
                    .await
                    .unwrap()
                    .unwrap()
                    .timestamp;
            }
            println!("{:?}, {:?}", event, meta);
            match event {
                ExactlyEvents::MaxFuturePoolsSetFilter(data) => {
                    self.markets
                        .entry(meta.address)
                        .or_insert_with_key(|key| FixedLender::new(*key))
                        .max_future_pools = data.new_max_future_pools.as_u32() as u8;
                }

                ExactlyEvents::OracleSetFilter(data) => {
                    self.oracle = ExactlyOracle::new(data.new_oracle, self.client.clone());
                    self.update_prices(meta.block_number).await?;
                    self.contracts_to_listen
                        .entry(ContractKey {
                            address: (*self.auditor).address(),
                            kind: ContractKeyKind::Oracle,
                        })
                        .or_insert(data.new_oracle);
                    return Ok((
                        meta.block_number,
                        meta.transaction_index.as_u64() as i128,
                        meta.log_index.as_u128() as i128,
                    ));
                }

                ExactlyEvents::AccumulatedEarningsSmoothFactorSetFilter(data) => {
                    self.markets
                        .entry(meta.address)
                        .or_insert_with_key(|key| FixedLender::new(*key))
                        .accumulated_earnings_smooth_factor =
                        data.new_accumulated_earnings_smooth_factor;
                }

                ExactlyEvents::MarketListedFilter(data) => {
                    let mut market = self
                        .markets
                        .entry(data.fixed_lender)
                        .or_insert_with_key(|key| FixedLender::new(*key));

                    market.decimals = data.decimals;
                    market.smart_pool_fee_rate = self.sp_fee_rate;
                    market.listed = true;
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
                    self.borrowers
                        .entry(data.owner)
                        .or_insert_with(|| Account::new(data.owner, &self.markets))
                        .deposit(&data, &meta.address);
                }
                ExactlyEvents::WithdrawFilter(data) => {
                    self.borrowers
                        .entry(data.owner)
                        .or_insert_with(|| Account::new(data.owner, &self.markets))
                        .withdraw(&data, &meta.address);
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
                ExactlyEvents::LiquidateBorrowFilter(data) => {
                    self.borrowers
                        .entry(data.borrower)
                        .or_insert_with(|| Account::new(data.borrower, &self.markets))
                        .liquidate_borrow(data, &meta.address);
                }
                ExactlyEvents::AssetSeizedFilter(data) => {
                    self.borrowers
                        .entry(data.borrower)
                        .or_insert_with(|| Account::new(data.borrower, &self.markets))
                        .asset_seized(data, &meta.address);
                }

                ExactlyEvents::AdjustFactorSetFilter(data) => {
                    self.markets
                        .entry(data.fixed_lender)
                        .or_insert_with_key(|key| FixedLender::new(*key))
                        .adjust_factor = data.new_adjust_factor;
                }

                ExactlyEvents::PenaltyRateSetFilter(data) => {
                    self.markets
                        .entry(meta.address)
                        .or_insert_with_key(|key| FixedLender::new(*key))
                        .penalty_rate = data.new_penalty_rate;
                }

                ExactlyEvents::MarketEnteredFilter(data) => {
                    self.borrowers
                        .entry(data.account)
                        .or_insert_with(|| Account::new(data.account, &self.markets))
                        .set_collateral(&data.fixed_lender);
                }

                ExactlyEvents::MarketExitedFilter(data) => {
                    self.borrowers
                        .entry(data.account)
                        .or_insert_with(|| Account::new(data.account, &self.markets))
                        .unset_collateral(&data.fixed_lender);
                }

                ExactlyEvents::SpFeeRateSetFilter(data) => {
                    self.sp_fee_rate = data.sp_fee_rate;
                    for market in self.markets.values_mut() {
                        market.smart_pool_fee_rate = data.sp_fee_rate;
                    }
                }

                ExactlyEvents::AssetSourceSetFilter(data) => {
                    self.contracts_to_listen
                        .entry(ContractKey {
                            address: data.fixed_lender,
                            kind: ContractKeyKind::PriceFeed,
                        })
                        .or_insert_with(|| data.source);
                    self.markets
                        .entry(data.fixed_lender)
                        .or_insert_with_key(|key| FixedLender::new(*key))
                        .price_feed = data.source;
                    self.update_prices(meta.block_number).await?;
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
                        .or_insert_with_key(|key| FixedLender::new(*key))
                        .oracle_price = data.current.into_raw() * U256::exp10(10);
                }

                ExactlyEvents::MarketUpdatedFilter(data) => {
                    let market = self
                        .markets
                        .entry(meta.address)
                        .or_insert_with_key(|address| FixedLender::new(*address));
                    market.total_shares = data.smart_pool_shares;
                    market.smart_pool_assets = data.smart_pool_assets;
                    market.smart_pool_earnings_accumulator = data.smart_pool_earnings_accumulator;

                    if data.maturity != U256::zero() {
                        let pool = market.fixed_pools.entry(data.maturity).or_default();
                        pool.last_accrual = data.timestamp;
                        pool.unassigned_earnings = data.maturity_unassigned_earnings;
                    } else {
                        market.last_accumulated_earnings_accrual = data.timestamp;
                    }
                }

                _ => {
                    println!("Event not handled - {:?}", event);
                }
            }
        }
        Ok((U64::zero(), -1, -1))
    }

    // Updates information for new blocks
    pub async fn update(&mut self) -> Result<()> {
        // Gets the last block
        let to = self.client.provider().get_block_number().await?;

        println!("Block: {:?}", to);
        if self.last_sync == (to, -1, -1) {
            return Ok(());
        }

        let f = Filter::new()
            .from_block(self.last_sync.0)
            .to_block(&to)
            .address(
                self.contracts_to_listen
                    .values()
                    .cloned()
                    .collect::<Vec<Address>>(),
            );
        let logs = self.client.provider().get_logs(&f).await?;
        println!(
            "Updating info from {} to {}",
            &(self.last_sync.0),
            &to,
            // &logs
        );
        let events = logs
            .into_iter()
            .map(|log| {
                let meta = LogMeta::from(&log);
                let result = ExactlyEvents::decode_log(&RawLog {
                    topics: log.topics,
                    data: log.data.to_vec(),
                });
                if let Err(_) = &result {
                    println!("{:?}", meta);
                }
                let event = result?;
                Ok((event, meta))
            })
            .collect::<Result<_>>()?;

        self.last_sync = self.handle_events(events, self.last_sync).await?;
        if let (_, -1, -1) = self.last_sync {
            self.last_sync.0 = to + 1
        } else {
            return Ok(());
        }
        // let previewer_borrowers = self.multicall_previewer(U64::from(&to)).await;
        let to_timestamp = self.provider.get_block(to).await?.unwrap().timestamp;
        if (*self.oracle).address() != Address::zero() {
            self.update_prices(to).await?;
        }

        println!("Final compare");
        self.compare_positions(to, to_timestamp).await?;

        let mut liquidations: HashMap<Address, Account> = HashMap::new();
        for (address, borrower) in self.borrowers.iter_mut() {
            let hf = Self::compute_hf(&mut self.markets, borrower, to_timestamp);
            if let Ok(hf) = hf {
                if hf < U256::exp10(18) && borrower.debt() != U256::zero() {
                    liquidations.insert(address.clone(), borrower.clone());
                }
            }
        }
        self.liquidate(&liquidations).await;

        Ok(())
    }

    async fn liquidate(&self, liquidations: &HashMap<Address, Account>) {
        for (_, borrower) in liquidations {
            println!("Liquidating borrower {:?}", borrower);
            if let Some(address) = &borrower.fixed_lender_to_liquidate() {
                println!("Liquidating on fixed lender {:?}", address);

                let contract = Market::<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>::new(
                    *address,
                    Arc::clone(&self.client),
                );

                let func = contract
                    .liquidate(
                        borrower.address(),
                        U256::MAX,
                        U256::MAX,
                        borrower.seizable_collateral().unwrap(),
                    )
                    .gas(6_666_666);
                let tx = func.send().await.unwrap();
                let receipt = tx.await.unwrap();
                println!("Liquidation tx {:?}", receipt);
            }
        }
    }

    async fn multicall_previewer(&self, block: U64) -> HashMap<Address, Vec<MarketAccount>> {
        let mut skip: usize = 0;
        let batch = 1000;
        let mut positions = HashMap::<Address, Vec<MarketAccount>>::new();
        while skip < self.borrowers.len() {
            let mut updated_waiting_data: Vec<Address> = Vec::new();
            let mut responses: Vec<Vec<MarketAccount>> = Vec::new();
            for borrower in self.borrowers.keys().skip(skip).take(batch) {
                updated_waiting_data.push(borrower.clone());

                let method = self.previewer.accounts(borrower.clone());
                let method = method.block(block);
                responses.push(method.call().await.unwrap());
            }

            let mut borrowers_updated = updated_waiting_data.iter();
            for payload in responses {
                let borrower = borrowers_updated
                    .next()
                    .expect("Number of self.borrowers and responses doesn't match");

                if positions.contains_key(&borrower) {
                    *positions.get_mut(borrower).unwrap() = payload;
                } else {
                    positions.insert(borrower.clone(), payload);
                }
            }

            skip += batch;
        }
        positions
    }

    async fn compare_positions(&self, block: U64, timestamp: U256) -> Result<()> {
        let previewer_borrowers = &self.multicall_previewer(U64::from(&block)).await;
        let event_borrowers = &self.borrowers;
        println!("Comparing positions");
        println!("-------------------");
        for (account, previewer_borrower) in previewer_borrowers {
            let event_borrower = &event_borrowers[account];
            println!("Account: {:?}", account);
            println!("---");
            if previewer_borrower.len() != event_borrower.data().len() {
                println!(
                    "Number of fixed lenders doesn't match. Previewer: {:?}, Event: {:?}",
                    previewer_borrower.len(),
                    event_borrower.data().len()
                );
            } else if previewer_borrower.len() == 0 {
                println!("No markets.");
            } else {
                for previewer_account in previewer_borrower.iter() {
                    let market_address = previewer_account.market;

                    if let Some(event_account) = event_borrower.data().get(&market_address) {
                        println!("Market: {:?}", market_address);
                        let mut _fixed_lender_correct = true;

                        let contract = Market::new(market_address, self.client.clone());
                        let total_assets =
                            contract.total_assets().block(block).call().await.unwrap();

                        let total_assets_event =
                            self.markets[&market_address].total_assets(timestamp);

                        const INTERVAL: u64 = 4 * 7 * 86_400;

                        let latest = ((timestamp - (timestamp % INTERVAL)) / INTERVAL).as_u64();
                        let max_future_pools = self.markets[&market_address].max_future_pools;
                        println!(
                            "latest: {}, max_future_pools: {}, INTERVAL: {}, timestamp: {}",
                            latest, max_future_pools, INTERVAL, timestamp
                        );

                        if false {
                            let event_account_market = &self.markets[&market_address];
                            let event_account_fixed_pools = &event_account_market.fixed_pools;
                            for i in latest..=latest + max_future_pools as u64 {
                                let maturity = U256::from(i) * INTERVAL;
                                let maturity_result = contract
                                    .fixed_pools(maturity)
                                    .block(block)
                                    .call()
                                    .await
                                    .unwrap();

                                if event_account_fixed_pools.contains_key(&maturity) {
                                    let maturity_local =
                                        event_account_fixed_pools.get(&maturity).unwrap();
                                    println!("Maturity: {:?}", maturity);
                                    println!(
                                        "Maturity: {:?} {:?}",
                                        maturity_result.0, maturity_local.borrowed
                                    );
                                    println!(
                                        "Maturity: {:?} {:?}",
                                        maturity_result.1, maturity_local.supplied
                                    );
                                    println!(
                                        "Maturity: {:?} {:?}",
                                        maturity_result.2, maturity_local.unassigned_earnings
                                    );
                                    println!(
                                        "Maturity: {:?} {:?}",
                                        maturity_result.3, maturity_local.last_accrual
                                    );
                                    if maturity_result.0 != maturity_local.borrowed
                                        || maturity_result.1 != maturity_local.supplied
                                        || maturity_result.2 != maturity_local.unassigned_earnings
                                        || maturity_result.3 != maturity_local.last_accrual
                                    {
                                        // panic!("Maturity");
                                    }
                                } else {
                                    println!("Maturity not in events");
                                }
                            }
                        }

                        let market = self.markets.get(&market_address).unwrap();

                        if &previewer_account.smart_pool_assets
                            != &event_account.smart_pool_assets(&market, timestamp)
                        {
                            _fixed_lender_correct = false;
                            println!("  smart_pool_assets:");
                            println!("    Previewer: {:?}", &previewer_account.smart_pool_assets);
                            println!(
                                "    Event    : {:?}",
                                &event_account.smart_pool_assets(&market, timestamp)
                            );
                            println!("\nMarket: {:?}", event_account);
                            println!("total_assets: {}\n", market.total_assets(timestamp));

                            println!(
                                "
total_assets = {:?}",
                                total_assets
                            );

                            // panic!("Debugging");
                        }

                        if total_assets != total_assets_event {
                            println!("  event: {}", total_assets_event);
                            println!("  previewer: {}", total_assets);
                            panic!("Debugging");
                        }

                        if previewer_account.smart_pool_shares != event_account.smart_pool_shares {
                            _fixed_lender_correct = false;
                            println!("  smart_pool_shares:");
                            println!("    Previewer: {:?}", &previewer_account.smart_pool_shares);
                            println!("    Event    : {:?}", &event_account.smart_pool_shares);
                            panic!("Debugging");
                        }
                        if previewer_account.oracle_price != market.oracle_price {
                            _fixed_lender_correct = false;
                            println!("  oracle_price:");
                            println!("    Previewer: {:?}", &previewer_account.oracle_price);
                            println!("    Event    : {:?}", &market.oracle_price);
                            panic!("Debugging");
                        }
                        if U256::from(previewer_account.penalty_rate) != market.penalty_rate {
                            _fixed_lender_correct = false;
                            println!("  penalty_rate:");
                            println!("    Previewer: {:?}", &previewer_account.penalty_rate);
                            println!("    Event    : {:?}", &market.penalty_rate);
                        }
                        if U256::from(previewer_account.adjust_factor) != market.adjust_factor {
                            _fixed_lender_correct = false;
                            println!("  adjust_factor:");
                            println!("    Previewer: {:?}", &previewer_account.adjust_factor);
                            println!("    Event    : {:?}", &market.adjust_factor);
                            panic!("Debugging");
                        }
                        if previewer_account.decimals != market.decimals {
                            _fixed_lender_correct = false;
                            println!("  decimals:");
                            println!("    Previewer: {:?}", previewer_account.decimals);
                            println!("    Event    : {:?}", &market.decimals);
                            panic!("Debugging");
                        }
                        if previewer_account.is_collateral != event_account.is_collateral {
                            _fixed_lender_correct = false;
                            println!("  is_collateral:");
                            println!("    Previewer: {:?}", &previewer_account.is_collateral);
                            println!("    Event    : {:?}", &event_account.is_collateral);
                            panic!("Debugging");
                        }

                        let mut _supplies_correct = true;
                        for position in &previewer_account.maturity_supply_positions {
                            let total_supply =
                                event_account.maturity_supply_positions[&position.maturity];
                            let total = position.position.principal + position.position.fee;

                            if total != total_supply {
                                _supplies_correct = false;
                                println!("  supplies:");
                                println!("    Previewer: {:?}", &total);
                                println!("    Event    : {:?}", &total_supply);
                                panic!("Debugging");
                            }
                        }
                        // for (maturity, total) in event_account.maturity_supply_positions {
                        //     if previewer_account.maturity_supply_positions[&maturity] == None {
                        //         supplies_correct = false;
                        //         println!("    Maturity {:?} not found on previewer", maturity);
                        //         panic!("Debugging");
                        //     }
                        // }

                        let mut _borrows_correct = true;
                        for position in &previewer_account.fixed_borrow_positions {
                            let borrowed_total =
                                &event_account.maturity_borrow_positions[&position.maturity];
                            let total = position.position.principal + position.position.fee;
                            if total != *borrowed_total {
                                _borrows_correct = false;
                                println!("  borrows:");
                                println!("    Previewer: {:?}", &total);
                                println!("    Event    : {:?}", &borrowed_total);
                                panic!("Debugging");
                            }
                        }
                        // for (maturity, total) in event_account.maturity_borrow_positions() {
                        //     if previewer_account.borrow_at_maturity(&maturity) == None {
                        //         borrows_correct = false;
                        //         println!("    Maturity {:?} not found on previewer", maturity);
                        //         panic!("Debugging");
                        //     }
                        // }
                    } else {
                        println!(
                            "Fixed lender {:?} doesn't found in data generated by events.",
                            &previewer_account.market
                        );
                    }
                }
            }
            println!("-------------------\n");
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

    pub fn compute_hf(
        markets: &mut HashMap<Address, FixedLender>,
        account: &mut Account,
        timestamp: U256,
    ) -> Result<U256, Error> {
        let mut collateral: U256 = U256::zero();
        let mut debt: U256 = U256::zero();
        let mut seizable_collateral: (U256, Option<Address>) = (U256::zero(), None);
        let mut fixed_lender_to_liquidate: (U256, Option<Address>) = (U256::zero(), None);
        for (market_address, position) in account.positions.iter() {
            let market = markets.get_mut(market_address).unwrap();
            if position.is_collateral {
                let current_collateral = position.smart_pool_assets(market, timestamp)
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
                if *maturity < timestamp {
                    current_debt += (timestamp - maturity) * U256::from(market.penalty_rate)
                }
            }
            debt += current_debt * market.oracle_price / U256::exp10(market.decimals as usize);
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
