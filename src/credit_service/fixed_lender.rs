use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::str::FromStr;
use std::sync::Arc;

use async_recursion::async_recursion;
use ethers::abi::Contract;
use ethers::prelude::builders::ContractCall;
use ethers::prelude::*;
use ethers::{
    contract::{self as ethers_contract},
    core::{self as ethers_core, abi::Abi},
    providers::Middleware,
};
use serde_json::Value;

use crate::bindings::multicall_2::Multicall2;
use crate::bindings::{AddressProvider, FixedLenderEvents, Previewer};
use crate::config::config::str_to_address;
use crate::errors::LiquidationError;
use crate::path_finder::PathFinder;
use crate::price_oracle::oracle::PriceOracle;

use super::Borrower;

pub struct FixedLender<M> {
    contract: ethers_contract::Contract<M>,
    previewer: Arc<Previewer<M>>,
    auditor: Arc<AddressProvider<M>>,
    borrower_accounts: HashMap<Address, Borrower>,
    underlying_token: ethers_core::types::Address,
    multicall: Arc<Multicall2<M>>,
    pub allowed_tokens: Vec<ethers_core::types::Address>,
}

impl<M> std::ops::Deref for FixedLender<M> {
    type Target = ethers_contract::Contract<M>;
    fn deref(&self) -> &Self::Target {
        &self.contract
    }
}

impl<M: Middleware> FixedLender<M> {
    fn parse_abi(abi_path: &str) -> (Address, Abi) {
        //"node_modules/@exactly-finance/protocol/deployments/kovan/FixedLenderDAI.json"
        let file = File::open(abi_path).unwrap();
        let reader = BufReader::new(file);
        let contract: Value = serde_json::from_reader(reader).unwrap();
        let (contract, abi): (Value, Value) = if let Value::Object(abi) = contract {
            (abi["address"].clone(), abi["abi"].clone())
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
        (contract, abi)
    }

    #[async_recursion]
    async fn load_events(
        &mut self,
        from_block: &U64,
        to_block: &U64,
    ) -> Vec<(FixedLenderEvents, LogMeta)> {
        println!("Query events");
        let events = self
            .contract
            .event_with_filter(Default::default())
            .from_block(from_block)
            .to_block(to_block)
            .query_with_meta()
            .await;
        // 31585419
        match events {
            Ok(result) => {
                println!(
                    "Events loaded: {} for contract {:?}",
                    result.len(),
                    self.contract.address()
                );
                result
            }
            Err(err) => {
                println!("Query err: {:?}", err);

                let mid_block = (from_block + to_block) / 2u64;
                if mid_block == *from_block || mid_block == *to_block {
                    panic!("range is already narrow");
                }

                let mut left_part = self.load_events(from_block, &mid_block).await;
                let mut right_part = self.load_events(&(mid_block + 1u64), to_block).await;
                left_part.append(&mut right_part);
                left_part
            }
        }
    }

    async fn update_accounts(
        &mut self,
        from_block: &U64,
        to_block: &U64,
        liquidation_candidates: &mut HashSet<Address>,
    ) {
        // let mut updated: HashSet<Address> = HashSet::new();

        let events = self.load_events(from_block, to_block).await;

        let _counter: u64 = 0;
        let _oper_by_user: HashMap<Address, u64> = HashMap::new();

        println!("Credit account: {}", self.address());

        for event in events {
            println!("event: {:?}", event);
            match &event.0 {
                FixedLenderEvents::RoleGrantedFilter(_data) => {}
                FixedLenderEvents::RoleAdminChangedFilter(_data) => {}
                FixedLenderEvents::RoleRevokedFilter(_data) => {}
                FixedLenderEvents::TransferFilter(data) => {
                    if data.from != Address::zero() {
                        liquidation_candidates.insert(data.from);
                    }
                    if data.to != Address::zero() {
                        liquidation_candidates.insert(data.to);
                    }
                }
                FixedLenderEvents::DepositFilter(data) => {
                    liquidation_candidates.insert(data.owner);
                }
                FixedLenderEvents::WithdrawFilter(data) => {
                    liquidation_candidates.insert(data.owner);
                }
                FixedLenderEvents::ApprovalFilter(_data) => {}
                FixedLenderEvents::DepositAtMaturityFilter(data) => {
                    liquidation_candidates.insert(data.owner);
                }
                FixedLenderEvents::WithdrawAtMaturityFilter(data) => {
                    liquidation_candidates.insert(data.owner);
                }
                FixedLenderEvents::BorrowAtMaturityFilter(data) => {
                    liquidation_candidates.insert(data.borrower);
                }
                FixedLenderEvents::RepayAtMaturityFilter(data) => {
                    liquidation_candidates.insert(data.borrower);
                }
                FixedLenderEvents::LiquidateBorrowFilter(data) => {
                    liquidation_candidates.insert(data.borrower);
                }
                FixedLenderEvents::AssetSeizedFilter(data) => {
                    liquidation_candidates.insert(data.borrower);
                }
                FixedLenderEvents::AccumulatedEarningsSmoothFactorSetFilter(_data) => {}
                FixedLenderEvents::MaxFuturePoolsUpdatedFilter(_data) => {}
                FixedLenderEvents::SmartPoolEarningsAccruedFilter(_data) => {}
                FixedLenderEvents::PausedFilter(_data) => {}
                FixedLenderEvents::UnpausedFilter(_data) => {}
                FixedLenderEvents::MarketListedFilter(_) => {}
                FixedLenderEvents::MarketEnteredFilter(_) => {}
                FixedLenderEvents::MarketExitedFilter(_) => {}
                FixedLenderEvents::OracleSetFilter(_) => {}
                FixedLenderEvents::LiquidationIncentiveSetFilter(_) => {}
                FixedLenderEvents::BorrowCapUpdatedFilter(_) => {}
                FixedLenderEvents::AdjustFactorSetFilter(_) => {}
                FixedLenderEvents::InterestRateModelSetFilter(_) => {}
                FixedLenderEvents::PenaltyRateSetFilter(_) => {}
                FixedLenderEvents::SmartPoolReserveFactorSetFilter(_) => {}
                FixedLenderEvents::DampSpeedUpdatedFilter(_) => {}
            }
        }
        // println!("Got operations: {}", &counter);
        // println!("Got operations: {:?}", &oper_by_user.keys().len());
        println!("Fixed Lender: {:?}", &self.address());

        // let function = &self.previewer.abi().functions.get("accounts").unwrap()[0];

        dbg!(&liquidation_candidates);

        // let mut skip: usize = 0;
        // let batch = 1000;

        // while skip < updated.len() {
        //     let mut calls: Vec<(Address, Vec<u8>)> = Vec::new();

        //     let mut updated_waiting_data: Vec<Address> = Vec::new();
        //     for borrower in updated.clone().iter().skip(skip).take(batch) {
        //         updated_waiting_data.push(borrower.clone());
        //         let tokens = vec![Token::Address(*borrower)];
        //         let brw: Vec<u8> = (*function.encode_input(&tokens).unwrap()).to_owned();
        //         calls.push((self.previewer.address(), brw));
        //     }

        //     println!("multicall");

        //     let response = self.multicall.aggregate(calls).call().await.unwrap();

        //     println!("Response: {:?}", response);

        //     let responses = response.1;

        //     let mut updateds = updated_waiting_data.iter();
        //     for r in responses.iter() {
        //         println!("Iterates over responses");
        //         let payload: Vec<(
        //             Address,
        //             String,
        //             Vec<(U256, (U256, U256))>,
        //             Vec<(U256, (U256, U256))>,
        //             U256,
        //             U256,
        //             U256,
        //             U128,
        //             U128,
        //             u8,
        //             bool,
        //         )> = decode_function_data(function, r, false).unwrap();

        //         println!("payload[{}]: {:?}", payload.len(), &payload);

        //         // let _health_factor = 0; // payload.7.as_u64();
        //         let borrower = updateds
        //             .next()
        //             .expect("Number of borrowers and responses doesn't match");

        //         let borrower_account = Borrower::new(borrower.clone(), payload);

        //         if self.borrower_accounts.contains_key(&borrower) {
        //             // dbg!(data.unwrap().0);
        //             *self.borrower_accounts.get_mut(borrower).unwrap() = borrower_account;
        //         } else {
        //             self.borrower_accounts
        //                 .insert(borrower.clone(), borrower_account);
        //         }
        //     }

        //     skip += batch;
        //     println!("Accounts done: {} ", &skip);
        // }

        println!("\nTotal accs: {}", &self.borrower_accounts.len());
    }

    pub fn new(
        abi_path: &str,
        address: Option<Address>,
        client: Arc<M>,
        previewer: Arc<Previewer<M>>,
        auditor: Arc<AddressProvider<M>>,
    ) -> Self {
        let (address_parsed, abi) = Self::parse_abi(abi_path);
        let address = if let Some(address) = address {
            address
        } else {
            address_parsed
        };
        let contract = ethers::contract::Contract::new(address, abi, client.clone());
        let borrower_accounts = HashMap::new();
        let multicall = Multicall2::new(
            str_to_address("0x5ba1e12693dc8f9c48aad8770482f4739beed696"),
            client.clone(),
        );
        let multicall = Arc::new(multicall);
        let allowed_tokens = Vec::<Address>::new();
        Self {
            contract,
            previewer: Arc::clone(&previewer),
            auditor: Arc::clone(&auditor),
            borrower_accounts,
            underlying_token: Address::default(),
            multicall: Arc::clone(&multicall),
            allowed_tokens,
        }
    }

    pub fn total_assets(&self) -> ContractCall<M, ethers::types::U256> {
        self.contract
            .method("totalAssets", ())
            .expect("Method not found")
    }

    pub fn liquidate(
        &self,
        borrower: &Borrower,
        position_assets: U256,
        max_assets_allowed: U256,
    ) -> ContractCall<M, ()> {
        println!("Liquidate from: {:?}", self.contract.address());
        self.contract
            .method(
                "liquidate",
                (
                    borrower.borrower().clone(),
                    position_assets,
                    max_assets_allowed,
                    borrower.seizable_collateral().unwrap(),
                ),
            )
            .expect("Method not found")
    }

    pub async fn update<S, N>(
        &mut self,
        from_block: &U64,
        to_block: &U64,
        _price_oracle: &PriceOracle<N, S>,
        _path_finder: &PathFinder<M>,
        liquidation_candidates: &mut HashSet<Address>,
    ) -> Result<(), LiquidationError>
    where
        S: Signer,
        N: Middleware,
    {
        // self.credit_filter.update(from_block, to_block).await;

        self.update_accounts(from_block, to_block, liquidation_candidates)
            .await;

        // let new_ci = U256::from(0u64); // self.pool_service.get_new_ci().await;

        // println!("Fixed Lender: {:?}", &self.address());

        // let mut accs_to_liquidate: HashSet<Borrower> = HashSet::new();
        // for borrower in self.borrower_accounts.iter_mut() {
        //     let hf: U256 = borrower.1.compute_hf()?;

        //     if hf <= U256::exp10(18) && borrower.1.debt() != U256::zero() {
        //         // if self.added_to_job.contains_key(&borrower.1.borrower()) {
        //         //     let bad_debt_blocks = self.added_to_job[&ca.1.borrower] + 1;
        //         //     *self.added_to_job.get_mut(&ca.1.borrower).unwrap() = bad_debt_blocks;

        //         //     if bad_debt_blocks >= 5 && bad_debt_blocks % 50 == 5 {
        //         //         // Additional attempt to delete acc after 5 blocks
        //         //         if bad_debt_blocks == 5 {
        //         //             self.added_to_job.insert(*&ca.1.borrower, 0u32);
        //         //         }
        //         //         if bad_debt_blocks % 500 == 5 {
        //         //             self.ampq_service
        //         //                 .send(format!(
        //         //                     "BAD DEBT!: Credit manager: {:?}\nborrower: {:?}\nCharts:{}/accounts/{:?}/{:?}\nHf: {}",
        //         //                     &self.address, &ca.1.borrower,
        //         //                     &self.charts_url,
        //         //                     &self.address, &ca.1.borrower,
        //         //                     &hf
        //         //                 ))
        //         //                 .await;
        //         //         }
        //         //     }
        //         // } else {
        //         //     self.added_to_job.insert(*&ca.1.borrower, 0u32);
        //         // accs_to_liquidate.insert(borrower.1.borrower());
        //         accs_to_liquidate.insert(borrower.1.clone());
        //         // }
        //     } else {
        //         // self.added_to_job.remove(&ca.1.borrower);
        //     }
        // }

        // dbg!(&accs_to_liquidate);

        // println!("Starting liquidation process:");

        // for acc in accs_to_liquidate {
        //     // jobs.push(self.liquidations
        //     // (acc)).await?;
        //     // println!("Liquidating:\n{:?}", &acc);
        //     // let liquidate
        //     //  = self.liquidate
        //     // (&acc, acc.debt(), acc.debt());
        //     // let tx = liquidate
        //     // .send().await.unwrap();
        //     // println!("Liquidation tx: {:?}", &tx);
        //     // let receipt = tx.await;
        //     // println!("Liquidation receipt: {:?}", &receipt);
        //     // liquidation_candidates.insert(acc);
        // }
        Ok(())
    }
}
