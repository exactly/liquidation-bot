use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::{thread, time};

use ethers::abi::{RawLog, Token};
use ethers::contract::{Contract, Multicall};
use ethers::core::types::{Address, H160, U128, U256, U64};
use ethers::core::types::{Filter, ValueOrArray};
use ethers::prelude::builders::ContractCall;
use ethers::prelude::{
    abigen, decode_function_data, BlockNumber, Chain, ContractError, EthLogDecode, Http, LogMeta,
    Middleware, Provider, Signer, SignerMiddleware, StreamExt,
};

use crate::ampq_service::AmpqService;
use crate::bindings::multicall_2::Multicall2;
use crate::bindings::{AddressProvider, FixedLenderEvents, Previewer, PreviewerData};
use crate::config::config::str_to_address;
use crate::config::Config;
use crate::credit_service::{Borrower, FixedLender};
use crate::errors::LiquidationError;
use crate::errors::LiquidationError::NetError;
use crate::path_finder::PathFinder;
use crate::price_oracle::oracle::PriceOracle;
use crate::terminator_service::terminator::TerminatorService;
use crate::token_service::service::TokenService;

use super::BorrowerData;

abigen!(
    ExactlyOracle,
    // TODO make it works on kovan as well
    "lib/protocol/deployments/rinkeby/ExactlyOracle.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

pub struct CreditService<M: Middleware, S: Signer> {
    fixed_lenders: HashMap<Address, Arc<Mutex<FixedLender>>>,
    token_service: TokenService<SignerMiddleware<M, S>>,
    price_oracle: PriceOracle<M, S>,
    previewer: Arc<Previewer<SignerMiddleware<M, S>>>,
    client: Arc<SignerMiddleware<M, S>>,
    last_block_synced: U64,
    provider: Provider<Http>,
    path_finder: PathFinder<SignerMiddleware<M, S>>,
    ampq_service: AmpqService,
    bot_address: Address,
    terminator_service: TerminatorService<M, S>,
    chain_id: u64,
    etherscan: String,
    charts_url: String,
    liquidator_enabled: bool,
    config: Config,
    multicall: Arc<Multicall2<SignerMiddleware<M, S>>>,
}

impl<M: Middleware, S: Signer> CreditService<M, S> {
    pub async fn new(
        config: &Config,
        previewer: Arc<Previewer<SignerMiddleware<M, S>>>,
        client: Arc<SignerMiddleware<M, S>>,
        token_service: TokenService<SignerMiddleware<M, S>>,
        price_oracle: PriceOracle<M, S>,
        provider: Provider<Http>,
    ) -> CreditService<M, S> {
        let fixed_lenders = HashMap::new();
        let path_finder = PathFinder::new(&*config, client.clone());
        let ampq_service = AmpqService::new(config).await;
        let terminator_service = TerminatorService::new(
            &config.terminator_address,
            &config.terminator_flash_address,
            client.clone(),
            config.liquidator_enabled,
        )
        .await;
        let multicall = Multicall2::new(
            str_to_address("0x5ba1e12693dc8f9c48aad8770482f4739beed696"),
            client.clone(),
        );
        let multicall = Arc::new(multicall);

        CreditService {
            fixed_lenders,
            token_service,
            price_oracle,
            previewer,
            client,
            last_block_synced: U64::zero(),
            provider,
            path_finder,
            ampq_service,
            bot_address: *&config.terminator_address,
            terminator_service,
            chain_id: config.chain_id,
            etherscan: config.etherscan.clone(),
            charts_url: config.charts_url.clone(),
            liquidator_enabled: config.liquidator_enabled,
            config: config.clone(),
            multicall: Arc::clone(&multicall),
        }
    }

    pub async fn launch(
        &mut self,
        markets: Vec<H160>,
        auditor: Arc<AddressProvider<SignerMiddleware<M, S>>>,
        block_number: U64,
    ) {
        // println!("dc {:?}", &self.dc);
        // let cm_list = self
        //     .dc
        //     .get_credit_managers_list(self.dc.address())
        //     .call()
        //     .await
        //     .unwrap();

        // let addr = str_to_address("0x968f9a68a98819e2e6bb910466e191a7b6cf02f0".into());

        // for market in markets {
        //     let credit_manager = CreditManager::new(
        //         self.client.clone(),
        //         &market,
        //         DataCompressor::new(self.dc.address(), self.client.clone()),
        //         self.chain_id,
        //         self.ampq_service.clone(),
        //         self.charts_url.clone(),
        //     );
        // }
        println!(
            "Signer {:?} for fixed lenders",
            self.client.signer().address()
        );
        for market in markets {
            println!("Adding market {:?}", market);
            let fixed_lender = FixedLender::new(market);
            if !self.fixed_lenders.contains_key(&market) {
                self.fixed_lenders
                    .insert(market, Arc::new(Mutex::new(fixed_lender)));
            };
        }
        // println!("total assets {:?}", total_assets);
        // for cm in cm_list {
        //     let credit_manager: CreditManager<M, S> = CreditManager::new(
        //         self.client.clone(),
        //         &cm,
        //         DataCompressor::new(self.dc.address(), self.client.clone()),
        //         self.chain_id,
        //         self.ampq_service.clone(),
        //         self.charts_url.clone(),
        //     )
        //     .await;
        //     // if cm.0 == addr {
        //     self.credit_managers.push(credit_manager);
        //     // }
        // }

        // let tokens = self.get_tokens();
        // self.price_oracle.load_price_feeds(&tokens).await;
        // self.token_service.add_token(&tokens).await;

        // self.ampq_service
        //     .send(String::from("Liquidation bot started!"))
        //     .await;

        self.last_block_synced = block_number - 1u64;

        let auditor = Arc::clone(&auditor);

        self.update(&auditor).await;

        let watcher = self.client.clone();
        let mut on_block = watcher
            .watch_blocks()
            .await
            .map_err(ContractError::MiddlewareError::<SignerMiddleware<M, S>>)
            .unwrap()
            .stream();
        while on_block.next().await.is_some() {
            match self.update(&auditor).await {
                Err(e) => {
                    println!("{}", &e);
                    self.ampq_service.send(e.to_string()).await;
                }
                _ => {}
            }

            if !self.liquidator_enabled {
                println!("zzzzz...");
                let delay = time::Duration::from_secs(20);
                thread::sleep(delay);
            }
        }
    }

    // pub fn get_tokens(&self) -> HashSet<Address> {
    //     let mut set: HashSet<Address> = HashSet::new();

    //     for (_, fixed_lender) in self.fixed_lenders.iter() {
    //         for token in fixed_lender.allowed_tokens.iter() {
    //             set.insert(*token);
    //         }
    //     }

    //     set
    // }

    async fn update_preices(
        &self,
        oracle: Address,
        markets: &HashMap<Address, Arc<Mutex<FixedLender>>>,
        block: U64,
    ) {
        let contract = ExactlyOracle::new(oracle, self.client.clone());
        println!("\nUpdating prices:\n");
        for market in markets.values() {
            let mut market = market.lock().unwrap();
            let oracle_price = contract
                .get_asset_price(market.address())
                .block(block)
                .call()
                .await
                .unwrap();
            market.set_oracle_price(Some(oracle_price));
            println!("Market {:?} value {:?}", market.address(), oracle_price);
        }
    }

    async fn parse_events(
        &mut self,
        logs: Vec<(FixedLenderEvents, LogMeta)>,
    ) -> HashMap<Address, Borrower> {
        let mut borrowers: HashMap<Address, Borrower> = HashMap::new();
        let mut block = U64::from(0u64);
        let mut markets = HashMap::<Address, Arc<Mutex<FixedLender>>>::new();
        let mut oracle: Option<Address> = None;
        // let mut markets = self.fixed_lenders.clone();
        for (event, meta) in logs {
            print!("Fixed Lender {:?} - ", meta.address);
            if meta.block_number > block {
                if borrowers.len() > 0 {
                    let previewer_borrowers = borrowers.iter().map(|(k, _)| k.clone()).collect();
                    let previewer_borrowers = self
                        .multicall_previewer(
                            previewer_borrowers,
                            Arc::clone(&self.previewer),
                            Arc::clone(&self.multicall),
                            block,
                        )
                        .await;
                    println!("Does it validate the oracle?");
                    if let Some(oracle) = oracle {
                        println!("It will validate the oracle");
                        self.update_preices(oracle, &markets, block).await;
                    }

                    Self::compare_positions(&previewer_borrowers, &borrowers);
                }
                block = meta.block_number;
            }
            match event {
                // FixedLenderEvents::RoleGrantedFilter(_) => {}
                // FixedLenderEvents::RoleAdminChangedFilter(_) => {}
                // FixedLenderEvents::RoleRevokedFilter(_) => {}
                // FixedLenderEvents::MaxFuturePoolsUpdatedFilter(_) => {}
                // FixedLenderEvents::PausedFilter(_) => {}
                // FixedLenderEvents::UnpausedFilter(_) => {}
                // FixedLenderEvents::ApprovalFilter(_) => {}
                // FixedLenderEvents::LiquidationIncentiveUpdatedFilter(_) => {}
                // FixedLenderEvents::BorrowCapUpdatedFilter(_) => {}
                // FixedLenderEvents::InterestRateModelUpdatedFilter(_) => {}
                // FixedLenderEvents::SmartPoolReserveFactorUpdatedFilter(_) => {}
                // FixedLenderEvents::DampSpeedUpdatedFilter(_) => {}
                FixedLenderEvents::OracleSetFilter(data) => {
                    println!("OracleSetFilter\n{:?}\n", data);
                    oracle = Some(data.new_oracle);
                }

                FixedLenderEvents::SmartPoolEarningsAccruedFilter(data) => {
                    println!("SmartPoolEarningsAccruedFilter\n{:?}\n", data);
                }

                FixedLenderEvents::AccumulatedEarningsSmoothFactorSetFilter(data) => {
                    println!("AccumulatedEarningsSmoothFactorUpdatedFilter\n{:?}\n", data);
                }

                FixedLenderEvents::MarketListedFilter(data) => {
                    println!("MarketListedFilter\n{:?}\n", data);
                    if !markets.contains_key(&data.fixed_lender) {
                        let mut market = FixedLender::new(data.fixed_lender.clone());
                        markets.insert(data.fixed_lender.clone(), Arc::new(Mutex::new(market)));
                    }
                    let market = markets.get_mut(&data.fixed_lender).unwrap();
                    market.lock().unwrap().set_decimals(Some(data.decimals));
                }

                FixedLenderEvents::TransferFilter(data) => {
                    println!("Transfered\n{:?}\n", data);
                    if data.from != Address::zero() {}
                    if data.to != Address::zero() {}
                    // TODO se ambos enderecos forem diferentes de zero não é mint nem bur, entao tem tratarr
                }
                FixedLenderEvents::DepositFilter(data) => {
                    println!("Deposit\n{:?}\n", data);
                    let borrower = Self::create_borrower(&mut borrowers, &data.owner, &markets);
                    borrower.deposit(data, &meta.address);
                }
                FixedLenderEvents::WithdrawFilter(data) => {
                    println!("Withdraw\n{:?}\n", data);
                    if borrowers.contains_key(&data.owner) {
                        let borrower = borrowers.get_mut(&data.owner).unwrap();
                        borrower.withdraw(data, &meta.address);
                    }
                }
                FixedLenderEvents::DepositAtMaturityFilter(data) => {
                    println!("DepositAtMaturity\n{:?}\n", data);
                    let borrower = Self::create_borrower(&mut borrowers, &data.owner, &markets);
                    borrower.deposit_at_maturity(data, &meta.address);
                }
                FixedLenderEvents::WithdrawAtMaturityFilter(data) => {
                    println!("WithdrawAtMaturity\n{:?}\n", data);
                    if borrowers.contains_key(&data.owner) {
                        let borrower = borrowers.get_mut(&data.owner).unwrap();
                        borrower.withdraw_at_maturity(data, &meta.address);
                    }
                }
                FixedLenderEvents::BorrowAtMaturityFilter(data) => {
                    println!("BorrowAtMaturity\n{:?}\n", data);
                    if borrowers.contains_key(&data.borrower) {
                        let borrower = borrowers.get_mut(&data.borrower).unwrap();
                        borrower.borrow_at_maturity(data, &meta.address);
                    }
                }
                FixedLenderEvents::RepayAtMaturityFilter(data) => {
                    println!("RepayAtMaturity\n{:?}\n", data);
                    if borrowers.contains_key(&data.borrower) {
                        let borrower = borrowers.get_mut(&data.borrower).unwrap();
                        borrower.repay_at_maturity(data, &meta.address);
                    }
                }
                FixedLenderEvents::LiquidateBorrowFilter(data) => {
                    println!("LiquidateBorrow\n{:?}\n", data);
                    if borrowers.contains_key(&data.borrower) {
                        let borrower = borrowers.get_mut(&data.borrower).unwrap();
                        borrower.liquidate_borrow(data, &meta.address);
                    }
                }
                FixedLenderEvents::AssetSeizedFilter(data) => {
                    println!("AssetSeized\n{:?}\n", data);
                    if borrowers.contains_key(&data.borrower) {
                        let borrower = borrowers.get_mut(&data.borrower).unwrap();
                        borrower.asset_seized(data, &meta.address);
                    }
                }

                FixedLenderEvents::AdjustFactorSetFilter(data) => {
                    println!("AdjustFactorSetFilter\n{:?}\n", data);

                    if let Some(market) = markets.get_mut(&data.fixed_lender) {
                        market
                            .lock()
                            .unwrap()
                            .set_adjust_factor(Some(data.new_adjust_factor));
                    }
                }

                FixedLenderEvents::PenaltyRateSetFilter(data) => {
                    println!("PenaltyRateUpdatedFilter\n{:?}\n", data);

                    if !markets.contains_key(&meta.address) {
                        let mut market = FixedLender::new(meta.address);
                        markets.insert(meta.address, Arc::new(Mutex::new(market)));
                    }
                    let market = markets.get_mut(&meta.address).unwrap();

                    market
                        .lock()
                        .unwrap()
                        .set_penalty_rate(Some(data.new_penalty_rate));
                }

                FixedLenderEvents::MarketEnteredFilter(data) => {
                    println!("MarketEnteredFilter\n{:?}\n", data);
                    if borrowers.contains_key(&data.account) {
                        let borrower = borrowers.get_mut(&data.account).unwrap();
                        borrower.set_collateral(&data.fixed_lender);
                    }
                }

                FixedLenderEvents::MarketExitedFilter(data) => {
                    println!("MarketExitedFilter\n{:?}\n", data);
                    if borrowers.contains_key(&data.account) {
                        let borrower = borrowers.get_mut(&data.account).unwrap();
                        borrower.unset_collateral(&data.fixed_lender);
                    }
                }
                _ => {
                    println!("Event not handled - {:?}", event);
                }
            }
        }
        borrowers
    }

    // Updates information for new blocks
    pub async fn update(
        &mut self,
        auditor: &Arc<AddressProvider<SignerMiddleware<M, S>>>,
    ) -> Result<(), LiquidationError> {
        // Gets the last block
        let to = self
            .client
            .provider()
            .get_block_number()
            .await
            .map_err(|r| NetError(format!("cant get last block {}", r.to_string())))?;

        if self.last_block_synced == to {
            return Ok(());
        }

        let mut contracts_with_events: Vec<Address> =
            self.fixed_lenders.keys().map(|k| k.clone()).collect();
        contracts_with_events.push(auditor.address());
        let f = Filter::new()
            .from_block(self.last_block_synced + U64::from(1u64))
            .to_block(&to)
            .address(contracts_with_events);
        let logs = self
            .client
            .provider()
            .get_logs(&f)
            .await
            .map_err(|e| LiquidationError::NetError("Error getting logs".to_string()))?;
        println!(
            "Updating info from {} to {}",
            &(self.last_block_synced + U64::from(1u64)),
            &to,
            // &logs
        );

        let logs = logs
            .into_iter()
            .filter_map(|log| {
                let meta = LogMeta::from(&log);
                let event: Result<FixedLenderEvents, _> = EthLogDecode::decode_log(&RawLog {
                    topics: log.topics,
                    data: log.data.to_vec(),
                });
                if event.is_err() {
                    return None;
                }
                Some((event.unwrap(), meta))
            })
            .collect();

        let mut borrowers = self.parse_events(logs).await;
        let previewer_borrowers = borrowers.iter().map(|(k, _)| k.clone()).collect();
        let previewer_borrowers = self
            .multicall_previewer(
                previewer_borrowers,
                Arc::clone(&self.previewer),
                Arc::clone(&self.multicall),
                U64::from(&to),
            )
            .await;
        Self::compare_positions(&previewer_borrowers, &borrowers);
        // Load fresh prices from oracle
        // self.price_oracle.update_prices().await?;

        // let mut terminator_jobs: Vec<TerminatorJob> = Vec::new();

        // let mut liquidation_candidates = HashSet::<Address>::new();
        // Updates info
        // for (_, fixed_lender) in self.fixed_lenders.iter_mut() {
        //     fixed_lender
        //         .update(
        //             &(self.last_block_synced + U64::from(1u64)),
        //             &to,
        //             &self.price_oracle,
        //             &self.path_finder,
        //             &mut liquidation_candidates,
        //         )
        //         .await?
        // }

        // let liquiditations = self
        //     .update_borrowers_position(&mut liquidation_candidates)
        //     .await;
        let liquidations: HashMap<Address, Borrower> = HashMap::new();
        for (address, borrower) in borrowers.iter_mut() {
            println!("borrower {:?}", borrower);
            // let hf = borrower.compute_hf();
            // if let Ok(hf) = hf {
            //     if hf < U256::exp10(18) && borrower.debt() != U256::zero() {
            //         liquidations.insert(address.clone(), borrower.clone());
            //     }
            // }
        }
        self.liquidate(&liquidations).await;

        // println!("Terminator jobs : {}", &terminator_jobs.len());

        // for job in &terminator_jobs {
        //     let balance = self
        //         .token_service
        //         .get_balance(&job.underlying_token, &self.bot_address)
        //         .await;

        //     if self.liquidator_enabled {
        //         let mut msg: String;

        //         let mut terminator_type = 1;

        //         if balance < job.repay_amount {
        //             msg = format!(
        //                 "TERMINATOR 1 hasn't enough {} balance. Have {}, needed {}. Please refill {:?}\n\nStarting TERMINATOR_FLASH",
        //                 self.token_service.symbol(&job.underlying_token),
        //                 self.token_service.format_bn(&job.underlying_token, &balance),
        //                 self.token_service.format_bn(&job.underlying_token, &job.repay_amount),
        //                 self.bot_address
        //             );
        //             terminator_type = 2;
        //         } else {
        //             msg = "STARTING TERMINATOR 1".into();
        //         }

        //         println!("{}", &msg);
        //         self.ampq_service.send(msg).await;

        //         let receipt = self
        //             .terminator_service
        //             .liquidate(job, terminator_type)
        //             .await;

        //         match receipt {
        //             Ok(receipt) => {
        //                 msg = format!(
        //                     "{} account {:?} was successfully liquidated. TxHash: {}/tx/{:?} . Gas used: {:?}\nBlock number: {}",
        //                     self.token_service.symbol(&job.underlying_token),
        //                     &job.borrower,
        //                     &self.etherscan,
        //                     &receipt.transaction_hash,
        //                     &receipt.gas_used.unwrap(),
        //                     &receipt.block_number.unwrap().as_u64()
        //                 );
        //             }
        //             Err(err) => {
        //                 msg = format!(
        //                     "WARN: Cant liquidate\nCredit manager: {:?}\naccount {:?}\n{:?}",
        //                     &job.credit_manager, &job.borrower, err
        //                 );
        //             }
        //         }

        //         println!("{}", &msg);
        //         self.ampq_service.send(msg).await;
        //     } else {
        //         let msg = format!(
        //             "Liquidation required:\ncredit manager {}: {}/address/{:?}\nborrower: {:?}\namount needed: {}",
        //             self.token_service.symbol(&job.underlying_token),
        //             &self.etherscan,
        //             &job.credit_manager,
        //             &job.borrower,
        //             self.token_service.format_bn(&job.underlying_token, &job.repay_amount),
        //             // self.bot_address
        //         );

        //         self.ampq_service.send(msg).await;
        //     }
        // }

        // Updates the last block synced
        self.last_block_synced = to;
        Ok(())
    }

    async fn update_borrowers_position(
        &self,
        candidates: &mut HashSet<Address>,
    ) -> HashMap<Address, Borrower> {
        let function = &self.previewer.abi().functions.get("accounts").unwrap()[0];
        let mut skip: usize = 0;
        let batch = 1000;
        let mut liquidations = HashMap::<Address, Borrower>::new();
        while skip < candidates.len() {
            let mut calls: Vec<(Address, Address)> = Vec::new();

            let mut updated_waiting_data: Vec<Address> = Vec::new();
            for borrower in candidates.clone().iter().skip(skip).take(batch) {
                println!("Creating multicall for borrower {}", borrower);
                updated_waiting_data.push(borrower.clone());
                // let tokens = vec![Token::Address(borrower.clone())];
                // let brw: Vec<u8> = (*function.encode_input(&tokens).unwrap()).to_owned();
                let brw = borrower.clone();
                calls.push((self.previewer.address(), brw));
                // self.previewer
                //     .accounts(borrower.clone())
                //     .call()
                //     .await
                //     .unwrap();
            }

            println!("multicall");

            let response = self.multicall.aggregate(calls).call().await;

            if let Err(e) = response {
                println!("WARN: Cant get borrowers info {:?}", e);
                skip += batch;
                continue;
            }

            let response = response.unwrap();

            println!("Response: {:?}", response);

            let responses = response.1;

            let mut updateds = updated_waiting_data.iter();
            for r in responses.iter() {
                let payload: Vec<(
                    Address,
                    String,
                    U256,
                    U128,
                    U128,
                    u8,
                    u8,
                    bool,
                    U256,
                    U256,
                    Vec<(U256, (U256, U256))>,
                    Vec<(U256, (U256, U256))>,
                )> = decode_function_data(function, r, false).unwrap();

                // println!("payload[{}]: {:?}", payload.len(), &payload);

                // let _health_factor = 0; // payload.7.as_u64();
                let borrower = updateds
                    .next()
                    .expect("Number of borrowers and responses doesn't match");

                let mut borrower_account = Borrower::new_from_previewer(borrower.clone(), payload);

                let hf: U256 = borrower_account.compute_hf().unwrap();

                if hf > U256::exp10(18) || borrower_account.debt() == U256::zero() {
                    println!("Removing borrower {:?}", &borrower);
                    candidates.remove(&borrower);
                } else {
                    println!("Updating borrower {:?}", &borrower);
                    liquidations.insert(borrower.clone(), borrower_account);
                }
            }

            skip += batch;
            println!("Accounts done: {} ", &skip);
        }
        liquidations
    }

    async fn liquidate(&self, _liquidations: &HashMap<Address, Borrower>) {
        // for (_, borrower) in liquidations {
        //     println!("Liquidating borrower {:?}", borrower);
        //     if let Some(address) = &borrower.fixed_lender_to_liquidate() {
        //         println!("Liquidating on fixed lender {:?}", address);
        //         let liquidate = self.fixed_lenders[address].liquidate(
        //             &borrower,
        //             borrower.debt(),
        //             borrower.debt(),
        //         );
        //         let tx = liquidate.send().await.unwrap();
        //         let receipt = tx.await.unwrap();
        //         println!("Liquidation tx {:?}", receipt);
        //     }
        // }
    }

    async fn multicall_previewer(
        &self,
        borrowers: HashSet<Address>,
        previewer: Arc<Previewer<SignerMiddleware<M, S>>>,
        multicall: Arc<Multicall2<SignerMiddleware<M, S>>>,
        block: U64,
    ) -> HashMap<Address, Borrower> {
        let function = &previewer.abi().functions.get("accounts").unwrap()[0];
        let mut skip: usize = 0;
        let batch = 1000;
        let mut positions = HashMap::<Address, Borrower>::new();
        while skip < borrowers.len() {
            // let mut calls: Vec<(Address, Address)> = Vec::new();
            let mut calls = Vec::new();

            let mut updated_waiting_data: Vec<Address> = Vec::new();
            let mut responses: Vec<PreviewerData> = Vec::new();
            for borrower in borrowers.iter().skip(skip).take(batch) {
                updated_waiting_data.push(borrower.clone());
                // let tokens = vec![Token::Address(borrower.clone())];
                // let brw: Vec<u8> = (*function.encode_input(&tokens).unwrap()).to_owned();
                // let brw = Token::Address(borrower.clone());
                let brw = borrower.clone();
                // calls.push((previewer.address(), brw));

                let method = previewer.accounts(borrower.clone());
                let method = method.block(block);
                responses.push(method.call().await.unwrap());
                calls.push(method);
                // println!("Previewer{:?}", a);
            }

            println!("multicall");

            // let mut multicall = Multicall::new(self.client.clone(), None).await.unwrap();
            // for call in calls {
            //     multicall.add_call(call);
            // }
            // let responses = multicall.call::<Vec<PreviewerData>>().await.unwrap();

            // let response = multicall
            //     .aggregate(calls)
            //     // .block(block)
            //     .call()
            //     .await
            //     .unwrap();

            println!("Response: {:?}", responses);

            // let responses = response.1;

            let mut updateds = updated_waiting_data.iter();
            for payload in responses {
                println!("Iterates over responses");
                // let payload: Vec<PreviewerData> = decode_function_data(function, r, false).unwrap();

                println!("payload[{}]: {:?}", payload.len(), &payload);

                // let _health_factor = 0; // payload.7.as_u64();
                let borrower = updateds
                    .next()
                    .expect("Number of borrowers and responses doesn't match");

                let borrower_account = Borrower::new_from_previewer(borrower.clone(), payload);

                if positions.contains_key(&borrower) {
                    *positions.get_mut(borrower).unwrap() = borrower_account;
                } else {
                    positions.insert(borrower.clone(), borrower_account);
                }
            }

            skip += batch;
            println!("Accounts done: {} ", &skip);
        }
        positions
    }

    fn compare_positions(
        previewer: &HashMap<Address, Borrower>,
        events: &HashMap<Address, Borrower>,
    ) {
        println!("Comparing positions");
        println!("-------------------");
        for (_, previewer_borrower) in previewer {
            if let Some(event_borrower) = events.get(&previewer_borrower.borrower()) {
                println!("Borrower: {:?}", &previewer_borrower.borrower());
                println!("---");
                if previewer_borrower.data().len() != event_borrower.data().len() {
                    println!(
                        "Number of fixed lenders doesn't match. Previewer: {:?}, Event: {:?}",
                        previewer_borrower.data().len(),
                        event_borrower.data().len()
                    );
                } else if previewer_borrower.data().len() == 0 {
                    println!("No markets.");
                } else {
                    for (_, data) in previewer_borrower.data().iter() {
                        let market_address = data.fixed_lender().lock().unwrap().address();
                        if let Some(fixed_lender) = event_borrower.data().get(&market_address) {
                            println!("Fixed lender: {:?}", &fixed_lender.fixed_lender());
                            let mut fixed_lender_correct = true;
                            if &data.smart_pool_assets() != &fixed_lender.smart_pool_assets() {
                                fixed_lender_correct = false;
                                println!("  smart_pool_assets:");
                                println!("    Previewer: {:?}", &data.smart_pool_assets());
                                println!("    Event    : {:?}", &fixed_lender.smart_pool_assets());
                            }
                            if &data.smart_pool_shares() != &fixed_lender.smart_pool_shares() {
                                fixed_lender_correct = false;
                                println!("  smart_pool_shares:");
                                println!("    Previewer: {:?}", &data.smart_pool_shares());
                                println!("    Event    : {:?}", &fixed_lender.smart_pool_shares());
                            }
                            if &data.oracle_price() != &fixed_lender.oracle_price() {
                                fixed_lender_correct = false;
                                println!("  oracle_price:");
                                println!("    Previewer: {:?}", &data.oracle_price());
                                println!("    Event    : {:?}", &fixed_lender.oracle_price());
                            }
                            // if &data.penalty_rate() != &fixed_lender.penalty_rate() {
                            fixed_lender_correct = false;
                            println!("  penalty_rate:");
                            println!("    Previewer: {:?}", &data.penalty_rate());
                            println!("    Event    : {:?}", &fixed_lender.penalty_rate());
                            // }
                            // if &data.adjust_factor() != &fixed_lender.adjust_factor() {
                            fixed_lender_correct = false;
                            println!("  adjust_factor:");
                            println!("    Previewer: {:?}", &data.adjust_factor());
                            println!("    Event    : {:?}", &fixed_lender.adjust_factor());
                            // }
                            // if &data.decimals() != &fixed_lender.decimals() {
                            fixed_lender_correct = false;
                            println!("  decimals:");
                            println!("    Previewer: {:?}", &data.decimals());
                            println!("    Event    : {:?}", &fixed_lender.decimals());
                            // }
                            if &data.is_collateral() != &fixed_lender.is_collateral() {
                                fixed_lender_correct = false;
                                println!("  is_collateral:");
                                println!("    Previewer: {:?}", &data.is_collateral());
                                println!("    Event    : {:?}", &fixed_lender.is_collateral());
                            }

                            println!("  supplies:");
                            let mut supplies_correct = true;
                            for (maturity, total) in data.maturity_supply_positions() {
                                if let Some(supply) = fixed_lender.supply_at_maturity(&maturity) {
                                    if total != supply {
                                        supplies_correct = false;
                                        println!("    Previewer: {:?}", &total);
                                        println!("    Event    : {:?}", &supply);
                                    }
                                } else {
                                    supplies_correct = false;
                                    println!("    Maturity {:?} not found on event", maturity);
                                }
                            }
                            for (maturity, total) in fixed_lender.maturity_supply_positions() {
                                if data.supply_at_maturity(&maturity) == None {
                                    supplies_correct = false;
                                    println!("    Maturity {:?} not found on previewer", maturity);
                                }
                            }
                            if supplies_correct {
                                println!("    Supplies are correct.");
                            }

                            println!("  borrows:");
                            let mut borrows_correct = true;
                            for (maturity, total) in data.maturity_borrow_positions() {
                                if let Some(borrow) = fixed_lender.borrow_at_maturity(&maturity) {
                                    if total != borrow {
                                        borrows_correct = false;
                                        println!("    Previewer: {:?}", &total);
                                        println!("    Event    : {:?}", &borrow);
                                    }
                                } else {
                                    borrows_correct = false;
                                    println!("    Maturity {:?} not found on event", maturity);
                                }
                            }
                            for (maturity, total) in fixed_lender.maturity_borrow_positions() {
                                if data.borrow_at_maturity(&maturity) == None {
                                    borrows_correct = false;
                                    println!("    Maturity {:?} not found on previewer", maturity);
                                }
                            }
                            if borrows_correct {
                                println!("    Borrows are correct.");
                            }
                        } else {
                            println!(
                                "Fixed lender {:?} doesn't found in data generated by events.",
                                &data.fixed_lender()
                            );
                        }
                    }
                }
                println!("-------------------\n");
            } else {
                println!(
                    "Borrower {:?} not found in events",
                    previewer_borrower.borrower()
                );
            }
        }
    }

    fn create_borrower<'a>(
        borrowers: &'a mut HashMap<H160, Borrower>,
        borrower_address: &Address,
        markets: &HashMap<Address, Arc<Mutex<FixedLender>>>,
    ) -> &'a mut Borrower {
        let borrower = if borrowers.contains_key(&borrower_address) {
            println!("Creating borrower\n{:?}\n", &borrower_address);
            borrowers.get_mut(borrower_address).unwrap()
        } else {
            println!("Creating borrower\n{:?}\n", borrower_address);
            let mut borrower_markets = Vec::<BorrowerData>::new();
            for (_, market) in markets {
                let borrower_market = BorrowerData::new_with_market(Arc::clone(&market));
                borrower_markets.push(borrower_market);
            }
            let mut borrower =
                Borrower::new_borrower_markets(borrower_address.clone(), borrower_markets);
            // borrower.add_markets(marksets);
            borrowers.insert(borrower_address.clone(), borrower);
            borrowers.get_mut(borrower_address).unwrap()
        };
        borrower
    }
}
