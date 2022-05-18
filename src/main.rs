//
extern crate hex;

//
use std::convert::TryFrom;
use std::sync::Arc;

use anyhow::Result;
use ethers::prelude::*;

use crate::bindings::Previewer;
//
use crate::bindings::address_provider::AddressProvider;
use crate::config::Config;
use crate::credit_service::service::CreditService;
use crate::price_oracle::oracle::PriceOracle;
use crate::token_service::service::TokenService;

//
mod ampq_service;
mod bindings;
mod config;
mod credit_service;
mod errors;
mod path_finder;
mod price_oracle;
mod terminator_service;
mod token_service;

#[tokio::main]
async fn main() -> Result<()> {
    println!("Gearbox liquidation bot started!");

    let config = Config::default();
    println!("Address provider: {:?} ", &config.address_provider);

    dbg!(&config);

    let provider = Provider::<Http>::try_from(config.eth_provider_rpc.clone())?;

    // create a wallet and connect it to the provider
    let wallet = config.private_key.parse::<LocalWallet>()?;
    let w2 = wallet.with_chain_id(config.chain_id);

    println!("Signer address: {:?}", &w2.address());

    let client: ethers::prelude::SignerMiddleware<
        ethers::prelude::Provider<ethers::prelude::Http>,
        ethers::prelude::Wallet<ethers_core::k256::ecdsa::SigningKey>,
    > = SignerMiddleware::new(provider.clone(), w2);

    let client = Arc::new(client);

    // let file = File::open("node_modules/@exactly-finance/protocol/deployments/kovan/Auditor.json")?;
    // let reader = BufReader::new(file);
    // // let auditor = ethers::abi::Contract::load(reader).unwrap();
    // let auditor: Value = serde_json::from_reader(reader)?;
    // println!("Auditor: {:?}", auditor["address"]);

    // // for f in auditor.functions() {
    // //     println!("{:?}", f.signature());
    // // }

    // let auditor_address = if let Value::String(s) = &auditor["address"] {
    //     s
    // } else {
    //     panic!("Auditor is not a string")
    // };

    // let auditor_address = decode_hex(&auditor_address[2..]).unwrap();
    // let auditor_address = H160::from_slice(auditor_address.as_slice());

    let address_provider = AddressProvider::new(client.clone());

    let data_compressor_addr = address_provider.get_data_compressor().call().await.unwrap();

    let previewer = Previewer::new(
        "node_modules/@exactly-finance/protocol/deployments/kovan/Previewer.json",
        None,
        Arc::clone(&client),
    );

    println!("Data compressor: {:?}", previewer);

    let token_service = TokenService::new(client.clone());

    let price_oracle = PriceOracle::new(
        client.clone(),
        //address_provider.get_price_oracle().call().await.unwrap(),
        Address::default(),
    );

    let mut credit_service = CreditService::new(
        &config,
        Arc::new(previewer),
        client,
        token_service,
        price_oracle,
        provider,
    )
    .await;

    credit_service
        .launch(data_compressor_addr, Arc::new(address_provider))
        .await;

    Ok(())
}
