use std::sync::Arc;

use ethers::prelude::k256::ecdsa::SigningKey;
use ethers::prelude::{Http, LocalWallet, Provider, Signer, SignerMiddleware, Wallet};
use eyre::Result;

use crate::config::Config;
use crate::credit_service::CreditService;

mod bindings;
mod config;
mod credit_service;

async fn create_client(
    // wallet: &Wallet<SigningKey>,
    config: &Config,
    eth_provider_rpc: &String,
) -> Arc<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>> {
    loop {
        let provider_result = Provider::<Http>::try_from(eth_provider_rpc.clone());
        let provider = if let Ok(provider) = provider_result {
            provider
        } else {
            println!("It wasn't possible to connect to the provider");
            continue;
        };
        // create a wallet and connect it to the provider
        let wallet = config.private_key.parse::<LocalWallet>();
        if let Ok(wallet) = wallet {
            let wallet = wallet.with_chain_id(config.chain_id);
            let client = SignerMiddleware::new(provider.clone(), wallet.clone());
            let client = Arc::new(client);
            return client;
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("exactly liquidation bot started!");

    let config = Config::default();
    println!("Address provider: {:?} ", &config.address_provider);

    let eth_provider_rpc = config.eth_provider_rpc.clone();

    dbg!(&config);

    let mut credit_service: Option<CreditService<Provider<Http>, Wallet<SigningKey>>> = None;
    let mut update_client = false;
    let mut last_client = None;
    loop {
        if let Some(client) = &last_client {
            if let Some(service) = &mut credit_service {
                if update_client {
                    println!("Updating client");
                    service.update_client(Arc::clone(client), &config).await;
                    update_client = false;
                }
            } else {
                credit_service = Some(CreditService::new(Arc::clone(client), &config).await?);
            }
            if let Some(service) = &mut credit_service {
                match service.launch().await {
                    Ok(_) => break,
                    Err(e) => {
                        println!("error: {:?}", e);
                        update_client = true;
                    }
                }
            }
        } else {
            last_client = Some(create_client(&config, &eth_provider_rpc).await);
        }
    }
    Ok(())
}
