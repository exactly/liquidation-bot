use std::sync::Arc;

use ethers::prelude::k256::ecdsa::SigningKey;
use ethers::prelude::{Provider, Signer, SignerMiddleware, Wallet, Ws};
use eyre::Result;

use crate::config::Config;
use crate::credit_service::CreditService;

mod bindings;
mod config;
mod credit_service;
mod fixed_point_math;

async fn create_client(
    config: &Config,
    eth_provider_rpc: &String,
) -> Arc<SignerMiddleware<Provider<Ws>, Wallet<SigningKey>>> {
    loop {
        let provider_result = Provider::<Ws>::connect(eth_provider_rpc.clone()).await;
        let provider = if let Ok(provider) = provider_result {
            provider
        } else {
            println!("It wasn't possible to connect to the provider");
            continue;
        };
        let wallet = config.wallet.clone().with_chain_id(config.chain_id);
        return Arc::new(SignerMiddleware::new(provider, wallet));
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("exactly liquidation bot started!");

    let config = Config::default();
    println!("Address provider: {:?} ", &config.address_provider);

    let eth_provider_rpc = config.eth_provider_rpc.clone();

    dbg!(&config);

    let mut credit_service: Option<CreditService<Provider<Ws>, Wallet<SigningKey>>> = None;
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
                println!("CREATING CREDIT SERVICE");
                credit_service = Some(CreditService::new(Arc::clone(client), &config).await?);
            }
            if let Some(service) = credit_service {
                credit_service = match service.launch().await {
                    Ok(current_service) => {
                        println!("CREDIT SERVICE ERROR");
                        Some(current_service)
                    }
                    Err(e) => {
                        println!("CREDIT SERVICE ERROR");
                        // println!("error: {:?}", e);
                        update_client = true;
                        Some(e)
                    }
                }
            }
        } else {
            last_client = Some(create_client(&config, &eth_provider_rpc).await);
        }
    }
    // Ok(())
}
