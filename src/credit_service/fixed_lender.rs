use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::str::FromStr;
use std::sync::Arc;

use async_recursion::async_recursion;
use ethers::abi::Token;
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
use crate::terminator_service::terminator::TerminatorJob;

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
        let events = self
            .contract
            .event_with_filter(Default::default())
            .from_block(from_block)
            .to_block(to_block)
            .query_with_meta()
            .await;

        match events {
            Ok(result) => result,
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

    async fn update_accounts(&mut self, from_block: &U64, to_block: &U64) {
        let mut updated: HashSet<Address> = HashSet::new();

        let events = self.load_events(from_block, to_block).await;

        let _counter: u64 = 0;
        let _oper_by_user: HashMap<Address, u64> = HashMap::new();

        println!("Credit account: {}", self.address());

        for event in events {
            println!("event: {:?}", event);
            match &event.0 {
                FixedLenderEvents::DepositAtMaturityFilter(_data) => {}
                FixedLenderEvents::WithdrawAtMaturityFilter(_data) => {}
                FixedLenderEvents::BorrowAtMaturityFilter(data) => {
                    updated.insert(data.borrower);
                }
                FixedLenderEvents::RepayAtMaturityFilter(_data) => {}
                FixedLenderEvents::LiquidateBorrowFilter(_data) => {}
                FixedLenderEvents::AssetSeizedFilter(_data) => {}
                FixedLenderEvents::AccumulatedEarningsSmoothFactorUpdatedFilter(_data) => {}
                FixedLenderEvents::MaxFuturePoolsUpdatedFilter(_data) => {}
            }
        }
        // println!("Got operations: {}", &counter);
        // println!("Got operations: {:?}", &oper_by_user.keys().len());
        println!("\n\nUnderlying token: {:?}", &self.underlying_token);
        println!("\n\nCredit manager: {:?}", &self.address());
        println!("Update needed for accounts: {}", &updated.len());

        let function = &self
            .previewer
            .abi()
            .functions
            .get("accountLiquidity")
            .unwrap()[0];

        dbg!(&updated);

        let mut skip: usize = 0;
        let batch = 1000;

        while skip < updated.len() {
            let mut calls: Vec<(ethers_core::types::Address, Vec<u8>)> = Vec::new();

            for borrower in updated.clone().iter().skip(skip).take(batch) {
                let tokens = vec![
                    Token::Address(self.auditor.address()),
                    Token::Address(*borrower),
                ];
                let brw: Vec<u8> = (*function.encode_input(&tokens).unwrap()).to_owned();
                calls.push((self.auditor.address(), brw));
            }

            let response = self.multicall.aggregate(calls).call().await.unwrap();

            let responses = response.1;

            for r in responses.iter() {
                let payload: (ethers_core::types::U256, ethers_core::types::U256) =
                    decode_function_data(function, r, false).unwrap();

                let _health_factor = 0; // payload.7.as_u64();

                let borrower = Address::default();

                let borrower_account = Borrower::new(borrower, payload.0, payload.1);

                if self.borrower_accounts.contains_key(&borrower) {
                    // dbg!(data.unwrap().0);
                    *self.borrower_accounts.get_mut(&borrower).unwrap() = borrower_account;
                } else {
                    self.borrower_accounts.insert(borrower, borrower_account);
                }
            }

            skip += batch;
            println!("Accounts done: {} ", &skip);
        }

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
        let contract = Contract::new(address, abi, client.clone());
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

    pub async fn update<S, N>(
        &mut self,
        _from_block: &U64,
        _to_block: &U64,
        price_oracle: &PriceOracle<N, S>,
        _path_finder: &PathFinder<M>,
        _jobs: &mut Vec<TerminatorJob>,
    ) -> Result<(), LiquidationError>
    where
        S: Signer,
        N: Middleware,
    {
        // self.credit_filter.update(from_block, to_block).await;

        let from = ethers::prelude::U64::from(0u64);
        let to = ethers::prelude::U64::from(0u64);
        self.update_accounts(&from, &to).await;

        let new_ci = U256::from(0u64); // self.pool_service.get_new_ci().await;

        println!("Credit manager: {:?}", &self.address());

        let accs_to_liquidate: HashSet<Address> = HashSet::new();
        for borrower in self.borrower_accounts.iter_mut() {
            let hf = borrower.1.compute_hf(
                self.underlying_token,
                &new_ci,
                price_oracle,
                // &self.contract,
            )?;

            if hf < 10000 {
                // if self.added_to_job.contains_key(&ca.1.borrower) {
                //     let bad_debt_blocks = self.added_to_job[&ca.1.borrower] + 1;
                //     *self.added_to_job.get_mut(&ca.1.borrower).unwrap() = bad_debt_blocks;

                //     if bad_debt_blocks >= 5 && bad_debt_blocks % 50 == 5 {
                //         // Additional attempt to delete acc after 5 blocks
                //         if bad_debt_blocks == 5 {
                //             self.added_to_job.insert(*&ca.1.borrower, 0u32);
                //         }
                //         if bad_debt_blocks % 500 == 5 {
                //             self.ampq_service
                //                 .send(format!(
                //                     "BAD DEBT!: Credit manager: {:?}\nborrower: {:?}\nCharts:{}/accounts/{:?}/{:?}\nHf: {}",
                //                     &self.address, &ca.1.borrower,
                //                     &self.charts_url,
                //                     &self.address, &ca.1.borrower,
                //                     &hf
                //                 ))
                //                 .await;
                //         }
                //     }
                // } else {
                //     self.added_to_job.insert(*&ca.1.borrower, 0u32);
                //     accs_to_liquidate.insert(ca.1.borrower);
                // }
            } else {
                // self.added_to_job.remove(&ca.1.borrower);
            }
        }

        dbg!(&accs_to_liquidate);

        println!("Starting liquidation process:");

        for _acc in accs_to_liquidate {
            //jobs.push(self.liquidate(&acc, path_finder).await?);
        }
        Ok(())
    }
}