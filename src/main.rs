use std::collections::BTreeMap;
use std::panic;
use std::process;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ethers::prelude::k256::ecdsa::SigningKey;
use ethers::prelude::{Provider, Signer, SignerMiddleware, Wallet, Ws};
use ethers::providers::Http;
use eyre::Result;

mod account;
mod config;
mod exactly_events;
mod market;
mod network;
mod protocol;

mod fixed_point_math;
mod liquidation;

mod generate_abi;

pub use account::*;
pub use exactly_events::*;
use log::error;
use log::info;
pub use market::Market;
pub use protocol::Protocol;
pub use protocol::ProtocolError;
use sentry::integrations::log::SentryLogger;
use sentry::Breadcrumb;
use sentry::Level;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::config::Config;

type ExactlyProtocol = Protocol<Provider<Ws>, Provider<Http>, Wallet<SigningKey>>;

const RETRIES: usize = 10;

enum ConnectFrom {
    Ws(String),
    Https(String),
}

enum ConnectResult {
    Ws(Provider<Ws>),
    Https(Provider<Http>),
}

async fn connect_ws(provider: &str) -> Result<ConnectResult> {
    let provider = Provider::<Ws>::connect(provider).await?;
    Ok(ConnectResult::Ws(provider))
}

async fn connect_https(provider: &str) -> Result<ConnectResult> {
    let provider = Provider::<Http>::try_from(provider)?;
    Ok(ConnectResult::Https(provider))
}

async fn retry_connect(provider: ConnectFrom) -> ConnectResult {
    let mut counter = 0;
    loop {
        let (provider_url, provider_result) = match &provider {
            ConnectFrom::Ws(provider) => (provider.clone(), connect_ws(provider).await),
            ConnectFrom::Https(provider) => (provider.clone(), connect_https(provider).await),
        };
        match provider_result {
            Ok(provider) => {
                break provider;
            }
            Err(ref e) => {
                if counter == RETRIES {
                    let mut data = BTreeMap::new();
                    data.insert(
                        "error message".to_string(),
                        Value::String(format!("{:?}", e)),
                    );
                    data.insert("connecting to".to_string(), Value::String(provider_url));
                    sentry::add_breadcrumb(Breadcrumb {
                        ty: "error".to_string(),
                        category: Some("Trying to connect".to_string()),
                        level: Level::Error,
                        data,
                        ..Default::default()
                    });
                    panic!(
                        "Failed to connect to provider after {} retries\nerror:{:?}",
                        RETRIES, e
                    );
                }
                counter += 1;
                thread::sleep(Duration::from_secs(1));
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
}

async fn create_client(
    config: &Config,
) -> (
    Arc<SignerMiddleware<Provider<Ws>, Wallet<SigningKey>>>,
    Arc<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>,
) {
    let ConnectResult::Ws(provider_ws) = retry_connect(ConnectFrom::Ws(config.rpc_provider.clone())).await else {
        panic!("Failed to connect to provider after {} retries", RETRIES);
    };
    let ConnectResult::Https(provider_https) = retry_connect(ConnectFrom::Https(config.rpc_provider_relayer.clone())).await else {
        panic!("Failed to connect to provider after {} retries", RETRIES);
    };
    let wallet = config.wallet.clone().with_chain_id(config.chain_id);
    (
        Arc::new(SignerMiddleware::new(provider_ws, wallet.clone())),
        Arc::new(SignerMiddleware::new(provider_https, wallet)),
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut log_builder = pretty_env_logger::formatted_builder();
    log_builder.parse_filters("warn,info,debug");
    let logger = SentryLogger::with_dest(log_builder.build());

    log::set_boxed_logger(Box::new(logger)).unwrap();
    if cfg!(debug_assertions) {
        log::set_max_level(log::LevelFilter::Debug);
    } else {
        log::set_max_level(log::LevelFilter::Info);
    }

    panic::set_hook(Box::new(|panic_info| {
        let mut data = BTreeMap::new();
        data.insert(
            "message".to_string(),
            Value::String(format!("{:?}", panic_info)),
        );
        if let Some(location) = panic_info.location() {
            data.insert("message".to_string(), Value::String(location.to_string()));
        }

        sentry::add_breadcrumb(Breadcrumb {
            ty: "error".to_string(),
            category: Some("panic".to_string()),
            level: Level::Error,
            data,
            ..Default::default()
        });

        if let Some(client) = sentry::Hub::current().client() {
            client.close(Some(Duration::from_secs(2)));
        }
        process::abort();
    }));

    let config = Config::default();

    let _guard = config.sentry_dsn.clone().map(|sentry_dsn| {
        sentry::init((
            sentry_dsn,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                debug: true,
                attach_stacktrace: true,
                default_integrations: true,
                max_breadcrumbs: 1000,
                traces_sample_rate: 1.0,
                enable_profiling: true,
                profiles_sample_rate: 1.0,
                ..Default::default()
            },
        ))
    });

    info!("Liquidation bot v{} starting", env!("CARGO_PKG_VERSION"));
    dbg!(&config);

    let mut credit_service: Option<Arc<Mutex<ExactlyProtocol>>> = None;
    let mut update_client = false;
    let mut last_client = None;
    loop {
        if let Some((client, client_relayer)) = &last_client {
            if let Some(service) = &mut credit_service {
                if update_client {
                    info!("Updating client");
                    service
                        .lock()
                        .await
                        .update_client(Arc::clone(client), Arc::clone(client_relayer), &config)
                        .await;
                    update_client = false;
                }
            } else {
                info!("creating service");
                credit_service = Some(Arc::new(Mutex::new(
                    Protocol::new(Arc::clone(client), Arc::clone(client_relayer), &config).await?,
                )));
            }
            if let Some(service) = &credit_service {
                match Protocol::launch(Arc::clone(service)).await {
                    Ok(()) => {
                        break;
                    }
                    Err(e) => {
                        let mut data = BTreeMap::new();
                        data.insert(
                            "Protocol error".to_string(),
                            Value::String(format!("{:?}", e)),
                        );
                        match e {
                            ProtocolError::SignerMiddlewareError(e) => {
                                data.insert(
                                    "Connection error".to_string(),
                                    Value::String(format!("{:?}", e)),
                                );
                                sentry::add_breadcrumb(Breadcrumb {
                                    ty: "error".to_string(),
                                    category: Some("Connection".to_string()),
                                    level: Level::Error,
                                    data,
                                    ..Default::default()
                                });

                                update_client = true;
                                last_client = None;
                            }
                            _ => {
                                sentry::add_breadcrumb(Breadcrumb {
                                    ty: "error".to_string(),
                                    category: Some("General error".to_string()),
                                    level: Level::Error,
                                    data,
                                    ..Default::default()
                                });

                                error!("ERROR");
                                break;
                            }
                        }
                    }
                }
            }
        } else {
            last_client = Some(create_client(&config).await);
        }
    }
    Ok(())
}
