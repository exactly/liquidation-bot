extern crate dotenv;
use ethers::prelude::{
    coins_bip39::English, k256::ecdsa::SigningKey, Address, MnemonicBuilder, Wallet,
};
use std::env;
use std::fmt::Debug;

#[derive(Debug, Clone)]
pub struct Config {
    pub chain_id: u64,
    pub chain_id_name: String,
    pub wallet: Wallet<SigningKey>,
    pub eth_provider_rpc: String,
    pub address_provider: Address,
    pub path_finder: Address,
    pub terminator_address: Address,
    pub terminator_flash_address: Address,
    pub ampq_addr: String,
    pub ampq_router_key: String,
    pub etherscan: String,
    pub liquidator_enabled: bool,
    pub comparison_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        dotenv::from_filename(".env").ok();
        let chain_id = get_env_or_throw("CHAIN_ID")
            .parse::<u64>()
            .expect("CHAIN_ID is not number");
        let address_provider = Address::zero();

        let wallet = MnemonicBuilder::<English>::default()
            .phrase(env::var("MNEMONIC").unwrap().as_str())
            .build()
            .unwrap();
        let path_finder = Address::zero();
        let ampq_addr = env::var("CLOUDAMQP_URL").unwrap_or("".into());
        let ampq_router_key = env::var("CLOUDAMQP_ROUTER").unwrap_or("".into());
        let terminator_address = Address::zero();
        let terminator_flash_address = Address::zero();
        let comparison_enabled: bool = env::var("COMPARISON_ENABLED")
            .unwrap_or("parse".into())
            .parse::<bool>()
            .unwrap_or(false);

        let (chain_id_name, eth_provider_rpc, etherscan) = match chain_id {
            1 => (
                "mainnet",
                get_env_or_throw("MAINNET_NODE"),
                "https://etherscan.io",
            ),
            4 => (
                "rinkeby",
                get_env_or_throw("RINKEBY_NODE"),
                "https://rinkeby.etherscan.io",
            ),
            1337 => (
                "fork",
                get_env_or_throw("FORK_NODE"),
                "https://etherscan.io",
            ),

            _ => {
                panic!("Unknown network!")
            }
        };

        let liquidator_enabled = if env::var("LIQUIDATOR_ENABLED").unwrap_or("".into()) == "true" {
            true
        } else {
            false
        };

        Config {
            chain_id,
            chain_id_name: chain_id_name.into(),
            address_provider,
            wallet,
            eth_provider_rpc,
            path_finder,
            ampq_addr,
            ampq_router_key,
            terminator_address,
            terminator_flash_address,
            etherscan: etherscan.into(),
            liquidator_enabled,
            comparison_enabled,
        }
    }
}

fn get_env_or_throw(env: &str) -> String {
    env::var(env).expect(format!("No {}", env).as_str())
}
