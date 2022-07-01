use std::sync::Arc;

use ethers::prelude::k256::ecdsa::SigningKey;
use ethers::prelude::{LocalWallet, Provider, SignerMiddleware, Wallet, Ws};
use eyre::Result;

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

    let provider = Provider::<Ws>::connect(config.eth_provider_rpc.clone()).await?;

    // create a wallet and connect it to the provider
    let wallet = config.private_key.parse::<LocalWallet>()?;

    let client: SignerMiddleware<Provider<Ws>, Wallet<SigningKey>> =
        SignerMiddleware::new(provider.clone(), wallet);

    let client = Arc::new(client);

    let mut credit_service = CreditService::new(client, config);

    credit_service.launch().await?;

    Ok(())
}
