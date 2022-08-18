pub mod borrower;
// pub mod credit_manager;
pub mod market;
pub mod service;

pub use borrower::*;
// pub use credit_manager::CreditManager;
pub use market::Market;
pub use service::CreditService;

use ethers::prelude::abigen;

abigen!(
    Market,
    "node_modules/@exactly-protocol/protocol/deployments/rinkeby/MarketWETH.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    Previewer,
    "node_modules/@exactly-protocol/protocol/deployments/rinkeby/Previewer.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    ExactlyOracle,
    "node_modules/@exactly-protocol/protocol/deployments/rinkeby/ExactlyOracle.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    Auditor,
    "node_modules/@exactly-protocol/protocol/deployments/rinkeby/Auditor.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    InterestRateModel,
    "node_modules/@exactly-protocol/protocol/deployments/rinkeby/InterestRateModel.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    PriceFeed,
    "node_modules/@chainlink/contracts/abi/v0.8/AggregatorV2V3Interface.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    AggregatorProxy,
    "node_modules/@chainlink/contracts/abi/v0.7/AggregatorProxyInterface.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    Liquidator,
    "deployments/rinkeby/Liquidator.json",
    event_derives(serde::Deserialize, serde::Serialize)
);
