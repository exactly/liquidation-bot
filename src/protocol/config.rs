extern crate dotenv;
use ethers::prelude::{coins_bip39::English, k256::ecdsa::SigningKey, MnemonicBuilder, Wallet};
use std::env;
use std::fmt::Debug;

#[derive(Debug, Clone)]
pub struct Config {
    pub chain_id: u64,
    pub chain_id_name: String,
    pub wallet: Wallet<SigningKey>,
    pub rpc_provider: String,
    pub rpc_provider_relayer: String,
    pub comparison_enabled: bool,
    pub token_pairs: String,
}

impl Default for Config {
    fn default() -> Self {
        dotenv::from_filename(".env").ok();
        let chain_id = get_env_or_throw("CHAIN_ID")
            .parse::<u64>()
            .expect("CHAIN_ID is not number");

        let wallet = MnemonicBuilder::<English>::default()
            .phrase(env::var("MNEMONIC").unwrap().as_str())
            .build()
            .unwrap();

        let comparison_enabled: bool = env::var("COMPARISON_ENABLED")
            .unwrap_or("parse".into())
            .parse::<bool>()
            .unwrap_or(false);

        let (chain_id_name, rpc_provider, rpc_provider_relayer) = match chain_id {
            1 => (
                "mainnet",
                get_env_or_throw("MAINNET_NODE"),
                get_env_or_throw("MAINNET_NODE_RELAYER"),
            ),
            4 => (
                "rinkeby",
                get_env_or_throw("RINKEBY_NODE"),
                get_env_or_throw("RINKEBY_NODE_RELAYER"),
            ),
            1337 => (
                "fork",
                get_env_or_throw("FORK_NODE"),
                get_env_or_throw("FORK_NODE_RELAYER"),
            ),
            _ => {
                panic!("Unknown network!")
            }
        };

        let token_pairs = env::var("TOKEN_PAIRS").unwrap_or("".into());

        Config {
            chain_id,
            chain_id_name: chain_id_name.into(),
            wallet,
            rpc_provider,
            rpc_provider_relayer,
            comparison_enabled,
            token_pairs,
        }
    }
}

fn get_env_or_throw(env: &str) -> String {
    env::var(env).expect(format!("No {}", env).as_str())
}
