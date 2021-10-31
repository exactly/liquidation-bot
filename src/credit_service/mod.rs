pub mod borrower;
// pub mod credit_manager;
pub mod fixed_lender;
pub mod service;

pub use borrower::*;
// pub use credit_manager::CreditManager;
pub use fixed_lender::FixedLender;
pub use service::CreditService;

use ethers::prelude::abigen;

abigen!(
    Market,
    "lib/protocol/deployments/rinkeby/FixedLenderWETH.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    Previewer,
    "lib/protocol/deployments/rinkeby/Previewer.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    ExactlyOracle,
    "lib/protocol/deployments/rinkeby/ExactlyOracle.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    Auditor,
    "lib/protocol/deployments/rinkeby/Auditor.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    InterestRateModel,
    "lib/protocol/deployments/rinkeby/InterestRateModel.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    PriceFeed,
    "lib/chainlink/contracts/abi/v0.8/AggregatorV2V3Interface.json",
    event_derives(serde::Deserialize, serde::Serialize)
);
