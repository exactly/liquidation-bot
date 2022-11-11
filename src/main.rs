use std::panic;
use std::process;
use std::sync::Arc;

use ethers::prelude::k256::ecdsa::SigningKey;
use ethers::prelude::{Provider, Signer, SignerMiddleware, Wallet, Ws};
use ethers::providers::Http;
use eyre::Result;

mod account;
mod config;
mod exactly_events;
mod market;
mod protocol;

mod fixed_point_math;
mod liquidation;

mod generate_abi;

pub use account::*;
pub use exactly_events::*;
pub use market::Market;
pub use protocol::Protocol;

use crate::config::Config;

async fn create_client(
    config: &Config,
) -> (
    Arc<SignerMiddleware<Provider<Ws>, Wallet<SigningKey>>>,
    Arc<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>,
) {
    let provider_ws = loop {
        let provider_result = Provider::<Ws>::connect(config.rpc_provider.clone()).await;
        if let Ok(provider) = provider_result {
            break provider;
        }
    };
    let provider_https = loop {
        let provider_result = Provider::<Http>::try_from(config.rpc_provider_relayer.clone());
        if let Ok(provider) = provider_result {
            break provider;
        }
    };
    let wallet = config.wallet.clone().with_chain_id(config.chain_id);
    (
        Arc::new(SignerMiddleware::new(provider_ws, wallet.clone())),
        Arc::new(SignerMiddleware::new(provider_https, wallet)),
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("exactly liquidation bot started!");

    panic::set_hook(Box::new(|panic_info| {
        println!("panic: {:?}", panic_info);
        process::abort();
    }));

    let config = Config::default();

    let _guard = config.sentry_dsn.clone().map(|sentry_dsn| {
        sentry::init((
            sentry_dsn,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                ..Default::default()
            },
        ))
    });

    dbg!(&config);

    let mut credit_service: Option<Protocol<Provider<Ws>, Provider<Http>, Wallet<SigningKey>>> =
        None;
    let mut update_client = false;
    let mut last_client = None;
    loop {
        if let Some((client, client_relayer)) = &last_client {
            if let Some(service) = &mut credit_service {
                if update_client {
                    println!("Updating client");
                    service
                        .update_client(Arc::clone(client), Arc::clone(client_relayer), &config)
                        .await;
                    update_client = false;
                }
            } else {
                println!("CREATING CREDIT SERVICE");
                credit_service = Some(
                    Protocol::new(Arc::clone(client), Arc::clone(client_relayer), &config).await?,
                );
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
            last_client = Some(create_client(&config).await);
        }
    }
    // Ok(())
}
