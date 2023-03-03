extern crate dotenv;

use ethers::prelude::{coins_bip39::English, k256::ecdsa::SigningKey, MnemonicBuilder, Wallet};
use ethers::types::U256;
use ethers::utils::{self, ParseUnits};
use std::env;
use std::fmt::Debug;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Config {
    pub chain_id: u64,
    pub chain_id_name: String,
    pub wallet: Wallet<SigningKey>,
    pub rpc_provider: String,
    pub rpc_provider_relayer: String,
    pub comparison_enabled: bool,
    pub token_pairs: String,
    pub backup: u32,
    pub liquidate_unprofitable: bool,
    pub repay_offset: U256,
    pub sentry_dsn: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        dotenv::from_filename(".env").ok();
        let chain_id = get_env_or_throw("CHAIN_ID")
            .parse::<u64>()
            .expect("CHAIN_ID is not number");

        let (chain_id_name, rpc_provider, rpc_provider_relayer) = match chain_id {
            1 => {
                dotenv::from_filename(".env.mainnet").ok();
                (
                    "mainnet",
                    get_env_or_throw("MAINNET_NODE"),
                    get_env_or_throw("MAINNET_NODE_RELAYER"),
                )
            }
            5 => {
                dotenv::from_filename(".env.goerli").ok();
                (
                    "goerli",
                    get_env_or_throw("GOERLI_NODE"),
                    get_env_or_throw("GOERLI_NODE_RELAYER"),
                )
            }
            10 => {
                dotenv::from_filename(".env.optimism").ok();
                (
                    "optimism",
                    get_env_or_throw("OPTIMISM_NODE"),
                    get_env_or_throw("OPTIMISM_NODE_RELAYER"),
                )
            }
            420 => {
                dotenv::from_filename(".env.optimism-goerli").ok();
                (
                    "optimism-goerli",
                    get_env_or_throw("OPTIMISM_GOERLI_NODE"),
                    get_env_or_throw("OPTIMISM_GOERLI_NODE_RELAYER"),
                )
            }
            1337 => (
                "fork",
                get_env_or_throw("FORK_NODE"),
                get_env_or_throw("FORK_NODE_RELAYER"),
            ),
            _ => {
                panic!("Unknown network!")
            }
        };

        let wallet = MnemonicBuilder::<English>::default()
            .phrase(env::var("MNEMONIC").unwrap().as_str())
            .build()
            .unwrap();

        let comparison_enabled: bool = env::var("COMPARISON_ENABLED")
            .unwrap_or_else(|_| "parse".into())
            .parse::<bool>()
            .unwrap_or(false);

        let backup = env::var("BACKUP")
            .unwrap_or_else(|_| "0".into())
            .parse::<u32>()
            .unwrap_or(0);

        let token_pairs = env::var("TOKEN_PAIRS").unwrap_or_else(|_| "".into());

        let repay_offset = utils::parse_units(
            &env::var("REPAY_OFFSET").unwrap_or_else(|_| "0.001".into()),
            18,
        )
        .unwrap();
        let repay_offset = match repay_offset {
            ParseUnits::U256(repay_offset) => repay_offset,
            _ => U256::from(0),
        };

        let liquidate_unprofitable =
            Arc::new(env::var("LIQUIDATE_UNPROFITABLE").unwrap_or_else(|_| "false".into()))
                .parse::<bool>()
                .unwrap_or(false);

        let sentry_dsn = env::var("SENTRY_DSN").ok();

        Config {
            chain_id,
            chain_id_name: chain_id_name.into(),
            wallet,
            rpc_provider,
            rpc_provider_relayer,
            comparison_enabled,
            token_pairs,
            backup,
            liquidate_unprofitable,
            repay_offset,
            sentry_dsn,
        }
    }
}

fn get_env_or_throw(env: &str) -> String {
    env::var(env).unwrap_or_else(|_| panic!("No {}", env))
}
