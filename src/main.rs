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

    let eth_provider_rpc = config.eth_provider_rpc.clone();

    // create a wallet and connect it to the provider
    let wallet = config.private_key.parse::<LocalWallet>()?;

    dbg!(&config);

    let mut credit_service: Option<CreditService<Provider<Ws>, Wallet<SigningKey>>> = None;
    let update_client = false;
    loop {
        let provider_result = Provider::<Ws>::connect(eth_provider_rpc.clone()).await;
        let provider = if let Ok(provider) = provider_result {
            provider
        } else {
            println!("It wasn't possible to connect to the provider");
            continue;
        };

        let client: SignerMiddleware<Provider<Ws>, Wallet<SigningKey>> =
            SignerMiddleware::new(provider.clone(), wallet.clone());

        let client = Arc::new(client);
        if let Some(service) = &mut credit_service {
            if update_client {
                service.update_client(Arc::clone(&client), &config).await;
            }
            service.launch().await?;
        } else {
            credit_service = Some(CreditService::new(Arc::clone(&client), &config).await?);
        }
    }
}
