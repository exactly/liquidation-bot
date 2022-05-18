use std::fs::File;
use std::io::BufReader;
use std::str::FromStr;
use std::sync::Arc;

use ethers::prelude::builders::ContractCall;
use ethers::prelude::*;
use ethers::{
    contract::{self as ethers_contract, builders::Event, Lazy},
    core::{self as ethers_core, abi::Abi, types::*},
    providers::{self as ethers_providers, Middleware},
};
use serde_json::Value;

pub struct ExactlyOracle<M> {
    contract: ethers_contract::Contract<M>,
}

impl<M> std::ops::Deref for ExactlyOracle<M> {
    type Target = ethers_contract::Contract<M>;
    fn deref(&self) -> &Self::Target {
        &self.contract
    }
}

impl<M: Middleware> ExactlyOracle<M> {
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

    pub fn new(abi_path: &str, client: Arc<M>) -> Self {
        let (address, abi) = Self::parse_abi(abi_path);
        let contract = Contract::new(address, abi, client);
        Self { contract }
    }
}
