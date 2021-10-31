//
extern crate hex;

//
use std::convert::TryFrom;
use std::sync::Arc;

use anyhow::Result;
use ethers::prelude::k256::ecdsa::SigningKey;
use ethers::prelude::*;

use crate::config::Config;
use crate::credit_service::CreditService;

mod bindings;
mod config;
mod credit_service;

#[tokio::main]
async fn main() -> Result<()> {
    println!("exactly liquidation bot started!");

    let config = Config::default();
    println!("Address provider: {:?} ", &config.address_provider);

    dbg!(&config);

    let provider = Provider::<Http>::try_from(config.eth_provider_rpc.clone())?;

    // create a wallet and connect it to the provider
    let wallet = config.private_key.parse::<LocalWallet>()?;
    let w2 = wallet.with_chain_id(config.chain_id);

    println!("Signer address: {:?}", &w2.address());

    let client: SignerMiddleware<Provider<Http>, Wallet<SigningKey>> =
        SignerMiddleware::new(provider.clone(), w2);

    let client = Arc::new(client);

    let mut credit_service = CreditService::new(client, provider, config);

    credit_service.launch().await?;

    Ok(())
}
